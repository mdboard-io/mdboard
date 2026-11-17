//! Git state and branch operations for the 1.0 upstream iteration.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitState {
    pub branch: String,
    pub commit: String,
    pub dirty: bool,
}

/// Current branch, commit and dirty state.
pub fn state(root: &Path) -> Result<GitState> {
    Ok(GitState {
        branch: current_branch(root)?,
        commit: head(root)?,
        dirty: is_dirty(root)?,
    })
}

pub fn current_branch(root: &Path) -> Result<String> {
    let branch = run(root, &["branch", "--show-current"])?;
    if branch.is_empty() {
        bail!("detached HEAD: check out the upstream branch");
    }
    Ok(branch)
}

pub fn branch_exists(root: &Path, branch: &str) -> Result<bool> {
    git_ok(
        root,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
}

pub fn branches(root: &Path) -> Result<Vec<String>> {
    let output = run(
        root,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

pub fn head(root: &Path) -> Result<String> {
    run(root, &["rev-parse", "HEAD"])
}

pub fn rev_parse(root: &Path, revision: &str) -> Result<String> {
    run(root, &["rev-parse", revision])
}

pub fn is_clean(root: &Path) -> Result<bool> {
    Ok(run(root, &["status", "--porcelain"])?.is_empty())
}

pub fn is_dirty(root: &Path) -> Result<bool> {
    Ok(!is_clean(root)?)
}

pub fn dirty_paths(root: &Path) -> Result<Vec<String>> {
    let output = run_raw(root, &["status", "--porcelain"])?;
    Ok(output
        .lines()
        .filter_map(|line| line.get(3..))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect())
}

pub fn switch(root: &Path, branch: &str) -> Result<()> {
    run(root, &["switch", branch])?;
    Ok(())
}

pub fn switch_create(root: &Path, branch: &str) -> Result<()> {
    run(root, &["switch", "-c", branch])?;
    Ok(())
}

pub fn merge_ff_only(root: &Path, branch: &str) -> Result<()> {
    run(root, &["merge", "--ff-only", branch])?;
    Ok(())
}

pub fn tag(root: &Path, name: &str) -> Result<()> {
    run(root, &["tag", name])?;
    Ok(())
}

pub fn tag_exists(root: &Path, name: &str) -> Result<bool> {
    git_ok(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/tags/{name}"),
        ],
    )
}

/// Merge a branch with `--no-ff`; abort on conflict and report the files.
pub fn merge_no_ff(root: &Path, branch: &str, message: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["merge", "--no-ff", "--no-edit", "-m", message, branch])
        .current_dir(root)
        .output()
        .with_context(|| format!("cannot run git merge {branch}"))?;
    if !output.status.success() {
        let conflicts = Command::new("git")
            .args(["diff", "--name-only", "--diff-filter=U"])
            .current_dir(root)
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_default();
        let _ = Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(root)
            .output();
        let files = if conflicts.is_empty() {
            "unknown files".to_owned()
        } else {
            conflicts.replace('\n', ", ")
        };
        bail!(
            "git merge {branch} failed (conflicts: {files}): {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Stage all changes in the working tree.
pub fn stage_all(root: &Path) -> Result<()> {
    run(root, &["add", "-A"])?;
    Ok(())
}

/// Paths currently staged for the next commit.
pub fn staged_paths(root: &Path) -> Result<Vec<String>> {
    let output = run(root, &["diff", "--cached", "--name-only"])?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Stage everything and commit when there is a change.
pub fn commit_all(root: &Path, message: &str) -> Result<bool> {
    commit_all_as(root, message, None, None)
}

/// Stage everything and commit with an optional per-commit identity.
pub fn commit_all_as(
    root: &Path,
    message: &str,
    name: Option<&str>,
    email: Option<&str>,
) -> Result<bool> {
    commit_all_with_identities(root, message, Some((name, email)), Some((name, email)))
}

/// Stage everything and commit with distinct author and committer identities.
///
/// The author describes who made the semantic change; the committer describes
/// the process that wrote the commit. Each identity is `(name, email)` and may
/// be omitted to fall back to Git configuration.
pub fn commit_all_with_identities(
    root: &Path,
    message: &str,
    author: Option<(Option<&str>, Option<&str>)>,
    committer: Option<(Option<&str>, Option<&str>)>,
) -> Result<bool> {
    run(root, &["add", "-A"])?;
    if git_ok(root, &["diff", "--cached", "--quiet"])? {
        return Ok(false);
    }
    let mut command = Command::new("git");
    command.args(["commit", "-m", message]).current_dir(root);
    if let Some((name, email)) = author {
        if let Some(name) = name {
            command.env("GIT_AUTHOR_NAME", name);
        }
        if let Some(email) = email {
            command.env("GIT_AUTHOR_EMAIL", email);
        }
    }
    if let Some((name, email)) = committer {
        if let Some(name) = name {
            command.env("GIT_COMMITTER_NAME", name);
        }
        if let Some(email) = email {
            command.env("GIT_COMMITTER_EMAIL", email);
        }
    }
    let output = command.output()?;
    if !output.status.success() {
        bail!(
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(true)
}

fn run(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("cannot run git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn run_raw(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("cannot run git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn git_ok(root: &Path, args: &[&str]) -> Result<bool> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("cannot run git {}", args.join(" ")))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::commit_all_with_identities;
    use std::process::Command;

    #[test]
    fn commit_uses_distinct_author_and_committer() {
        let dir = tempfile::TempDir::new().unwrap();
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::fs::write(dir.path().join("work.txt"), "code\n").unwrap();
        commit_all_with_identities(
            dir.path(),
            "implement",
            Some((Some("Human Owner"), Some("owner@example.com"))),
            Some((Some("Coding Agent"), Some("agent@example.com"))),
        )
        .unwrap();
        let output = Command::new("git")
            .args(["log", "-1", "--format=%an|%ae|%cn|%ce"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "Human Owner|owner@example.com|Coding Agent|agent@example.com"
        );
    }
}
