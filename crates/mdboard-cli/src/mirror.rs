//! Private-to-public snapshot mirroring.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use mdboard_driver::process::ProcessLoaded;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct MirrorFile {
    #[serde(default)]
    profiles: BTreeMap<String, MirrorProfile>,
    #[serde(default)]
    precautions: Vec<Precaution>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct MirrorProfile {
    to: String,
    #[serde(default = "default_branch")]
    branch: String,
    #[serde(default)]
    ignore: Option<String>,
    #[serde(default)]
    keep_public: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    tags: Option<String>,
    #[serde(default)]
    snapshot: bool,
}

fn default_branch() -> String {
    "main".to_owned()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Precaution {
    id: String,
    text: String,
    #[serde(default)]
    profiles: Vec<String>,
    #[serde(default = "active")]
    status: String,
}

fn active() -> String {
    "active".to_owned()
}

#[derive(Debug, Default)]
pub struct MirrorOptions {
    pub profile: Option<String>,
    pub to: Option<String>,
    pub branch: Option<String>,
    pub dry_run: bool,
    pub push: bool,
    pub force: bool,
    pub confirm_first_push: bool,
    pub snapshot: bool,
    pub allow_existing: bool,
    pub tag: Option<String>,
    pub authorised_by: Option<String>,
    pub authorization: Option<String>,
}

struct Pattern {
    negated: bool,
    pattern: String,
}

pub fn run(root: &Path, options: &MirrorOptions) -> Result<()> {
    if !options.dry_run && !options.push {
        bail!("mirror requires --dry-run or --push");
    }
    if options.dry_run && options.push {
        bail!("choose either --dry-run or --push");
    }
    let loaded = ProcessLoaded::load(root)?;
    let branch = mdboard_git::current_branch(root)?;
    let allowed = [
        &loaded.policy.branches.nightly,
        &loaded.policy.branches.stable,
    ];
    if !allowed.contains(&&branch) {
        bail!(
            "mirror must run from the configured `{}` or `{}` branch, not `{branch}`",
            loaded.policy.branches.nightly,
            loaded.policy.branches.stable
        );
    }
    if !mdboard_git::is_clean(root)? {
        bail!("mirror requires a clean working tree");
    }
    let mirror_path = root.join(".mdboard/mirror.toml");
    let mirror: MirrorFile = if mirror_path.exists() {
        toml::from_str(&fs::read_to_string(&mirror_path)?)
            .with_context(|| format!("cannot parse {}", mirror_path.display()))?
    } else {
        MirrorFile::default()
    };
    let profile_name = options
        .profile
        .clone()
        .unwrap_or_else(|| "public".to_owned());
    let profile = mirror
        .profiles
        .get(&profile_name)
        .cloned()
        .with_context(|| format!("mirror profile `{profile_name}` not found"))?;
    let target = options.to.clone().unwrap_or(profile.to);
    if target.trim().is_empty() {
        bail!("mirror target is empty");
    }
    let target_branch = options.branch.clone().unwrap_or(profile.branch);
    let target_reachable = Command::new("git")
        .args(["ls-remote", &target])
        .output()
        .context("cannot check mirror target")?
        .status
        .success();
    if !target_reachable {
        bail!("mirror target is not reachable: {target}");
    }
    let heads = Command::new("git")
        .args(["ls-remote", "--heads", &target])
        .output()
        .context("cannot inspect mirror target")?;
    let target_has_history = !String::from_utf8_lossy(&heads.stdout).trim().is_empty();
    if target_has_history
        && !options.allow_existing
        && !receipt_exists(root, &profile_name, &target)
    {
        bail!(
            "mirror target already has history; rerun with --allow-existing after owner approval, or publish from a target with a prior receipt"
        );
    }
    let ignore_path = root.join(profile.ignore.as_deref().unwrap_or(".mdboardignore"));
    if !ignore_path.is_file() {
        bail!("ignore file is missing: {}", ignore_path.display());
    }
    let patterns = parse_ignore(&fs::read_to_string(&ignore_path)?);

    let mut included = Vec::new();
    let mut excluded = Vec::new();
    collect(root, Path::new(""), &patterns, &mut included, &mut excluded)?;
    let secrets = scan_secrets(root, &included)?;
    if !secrets.is_empty() {
        for finding in &secrets {
            eprintln!("secret finding: {finding}");
        }
        bail!("mirror refused: secret findings must be excluded or removed");
    }
    let precautions: Vec<_> = mirror
        .precautions
        .iter()
        .filter(|precaution| {
            precaution.status == "active"
                && (precaution.profiles.is_empty()
                    || precaution.profiles.iter().any(|name| name == &profile_name))
        })
        .collect();
    println!("mirror profile: {profile_name}");
    println!("source branch: {branch}");
    println!("target: {target} ({target_branch})");
    println!("included files: {}", included.len());
    println!("excluded files: {}", excluded.len());
    for precaution in precautions {
        println!("precaution {}: {}", precaution.id, precaution.text);
    }
    if options.dry_run {
        for path in &included {
            println!("  include {}", path.display());
        }
        for path in &excluded {
            println!("  exclude {}", path.display());
        }
        return Ok(());
    }
    let authorised_by = options
        .authorised_by
        .as_deref()
        .context("public push requires --authorised-by owner")?;
    let authorization = options
        .authorization
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("public push requires --authorization")?;
    let actors =
        mdboard_core::actor::Actors::load_optional(&loaded.process_root().join("actors.toml"))?;
    let authoriser = actors
        .get(authorised_by)
        .context("authorising actor is not configured")?;
    if authoriser.kind != mdboard_core::actor::ActorKind::Human || !authoriser.has_role("owner") {
        bail!("public push must be authorised by a human owner");
    }
    if !options.confirm_first_push {
        bail!("first public push requires --confirm-first-push");
    }
    let remote_commit = push_snapshot(
        root,
        &included,
        &profile.keep_public,
        &target,
        &target_branch,
        options.force,
        profile.snapshot || options.snapshot,
        options.tag.as_deref(),
    )?;
    write_receipt(
        root,
        &profile_name,
        &target,
        &target_branch,
        &branch,
        &remote_commit,
        options.tag.as_deref(),
        authorised_by,
        authorization,
    )?;
    println!("mirror: verified {target_branch} at {remote_commit}");
    Ok(())
}

fn collect(
    root: &Path,
    relative: &Path,
    patterns: &[Pattern],
    included: &mut Vec<PathBuf>,
    excluded: &mut Vec<PathBuf>,
) -> Result<()> {
    let dir = root.join(relative);
    let mut entries = fs::read_dir(&dir)
        .with_context(|| format!("cannot read {}", dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let child = relative.join(name);
        let rel = child.to_string_lossy().replace('\\', "/");
        let hard_excluded = name == ".git"
            || ((name == ".mdboard" || name == ".mdboardignore")
                && relative.as_os_str().is_empty());
        if hard_excluded || is_excluded(&rel, patterns) {
            excluded.push(child);
            continue;
        }
        let path = root.join(&child);
        if path.is_dir() {
            collect(root, &child, patterns, included, excluded)?;
        } else if path.is_file() {
            included.push(child);
        }
    }
    Ok(())
}

fn parse_ignore(text: &str) -> Vec<Pattern> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (negated, pattern) = line
                .strip_prefix('!')
                .map_or((false, line), |pattern| (true, pattern));
            Pattern {
                negated,
                pattern: pattern
                    .trim_start_matches('/')
                    .trim_end_matches('/')
                    .to_owned(),
            }
        })
        .collect()
}

fn is_excluded(path: &str, patterns: &[Pattern]) -> bool {
    let mut excluded = false;
    for pattern in patterns {
        if pattern.pattern.is_empty() {
            continue;
        }
        let matches = path == pattern.pattern
            || path.starts_with(&format!("{}/", pattern.pattern))
            || (!pattern.pattern.contains('/')
                && path.split('/').any(|part| glob(&pattern.pattern, part)));
        if matches {
            excluded = !pattern.negated;
        }
    }
    excluded
}

fn glob(pattern: &str, text: &str) -> bool {
    fn inner(pattern: &[u8], text: &[u8]) -> bool {
        if pattern.is_empty() {
            return text.is_empty();
        }
        if pattern.starts_with(b"**") {
            let rest = &pattern[2..];
            return (0..=text.len()).any(|index| inner(rest, &text[index..]));
        }
        match pattern[0] {
            b'*' => (0..=text.len()).any(|index| inner(&pattern[1..], &text[index..])),
            b'?' => !text.is_empty() && inner(&pattern[1..], &text[1..]),
            byte => !text.is_empty() && byte == text[0] && inner(&pattern[1..], &text[1..]),
        }
    }
    inner(pattern.as_bytes(), text.as_bytes())
}

fn scan_secrets(root: &Path, included: &[PathBuf]) -> Result<Vec<String>> {
    let mut findings = Vec::new();
    for path in included {
        let display = path.to_string_lossy();
        let lower = display.to_lowercase();
        if lower.ends_with(".env")
            || lower.ends_with(".pem")
            || lower.ends_with(".key")
            || lower.ends_with(".p12")
        {
            findings.push(format!("{display}: sensitive filename"));
        }
        let Ok(bytes) = fs::read(root.join(path)) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        if text.lines().any(|line| {
            matches!(
                line.trim(),
                "-----BEGIN PRIVATE KEY-----"
                    | "-----BEGIN RSA PRIVATE KEY-----"
                    | "-----BEGIN OPENSSH PRIVATE KEY-----"
                    | "-----BEGIN EC PRIVATE KEY-----"
                    | "-----BEGIN PGP PRIVATE KEY BLOCK-----"
            )
        }) {
            findings.push(format!("{display}: contains a private key block"));
        }
        if text.lines().any(|line| {
            line.trim_start()
                .to_ascii_lowercase()
                .starts_with("aws_secret_access_key")
        }) {
            findings.push(format!("{display}: contains AWS_SECRET_ACCESS_KEY"));
        }
        for line in text.lines() {
            let trimmed = line.trim_start();
            let lower_line = trimmed.to_ascii_lowercase();
            for key in ["api_key", "password", "secret", "token"] {
                let Some(rest) = lower_line.strip_prefix(key) else {
                    continue;
                };
                let rest = rest.trim_start();
                let Some(value) = rest.strip_prefix('=') else {
                    continue;
                };
                let value = value.trim().trim_matches('"').trim_matches('\'').trim();
                if value.len() >= 16
                    && !value.contains(' ')
                    && !value.starts_with("document.")
                    && !value.starts_with("process.env")
                {
                    findings.push(format!("{display}: contains a long `{key}` assignment"));
                }
            }
        }
    }
    Ok(findings)
}

fn keep_matches(path: &str, patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|pattern| path == pattern || glob(pattern, path))
}

#[allow(clippy::too_many_arguments)]
fn push_snapshot(
    root: &Path,
    included: &[PathBuf],
    keep_public: &[String],
    target: &str,
    branch: &str,
    force: bool,
    snapshot: bool,
    tag: Option<&str>,
) -> Result<String> {
    if snapshot {
        return push_orphan_snapshot(root, included, keep_public, target, branch, tag);
    }
    let temp = tempfile::tempdir()?;
    let checkout = temp.path().join("target");
    let cloned = Command::new("git")
        .args(["clone", "--quiet", "--branch", branch, target])
        .arg(&checkout)
        .status()
        .context("cannot run git clone")?
        .success();
    if !cloned {
        fs::create_dir_all(&checkout)?;
        git(&checkout, &["init", "-q"])?;
        git(&checkout, &["checkout", "-q", "-B", branch])?;
        git(&checkout, &["remote", "add", "origin", target])?;
    }
    let mut preserved = Vec::new();
    for entry in fs::read_dir(&checkout)?.flatten() {
        if entry.file_name() == ".git" {
            continue;
        }
        if entry.path().is_file() {
            let rel = entry
                .path()
                .strip_prefix(&checkout)
                .unwrap_or(&entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            if keep_matches(&rel, keep_public) {
                preserved.push((rel, fs::read(entry.path())?));
            }
        }
        let path = entry.path();
        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    for relative in included {
        let destination = checkout.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(root.join(relative), &destination)?;
    }
    for (rel, bytes) in preserved {
        let path = checkout.join(&rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)?;
    }
    git(&checkout, &["add", "-A"])?;
    let status = Command::new("git")
        .current_dir(&checkout)
        .args(["status", "--porcelain"])
        .output()?;
    if String::from_utf8_lossy(&status.stdout).trim().is_empty() {
        println!("mirror: no changes");
        let remote = Command::new("git")
            .args([
                "ls-remote",
                "--heads",
                target,
                &format!("refs/heads/{branch}"),
            ])
            .output()?;
        return Ok(String::from_utf8_lossy(&remote.stdout)
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_owned());
    }
    git(
        &checkout,
        &[
            "-c",
            "user.name=mdboard-mirror",
            "-c",
            "user.email=mirror@mdboard.io",
            "commit",
            "-q",
            "-m",
            &format!("Mirror from {}", root.display()),
        ],
    )?;
    let refspec = format!("HEAD:{branch}");
    let mut args = vec!["push", "origin", refspec.as_str()];
    if force {
        args.push("--force");
    }
    git(&checkout, &args)?;
    let remote = Command::new("git")
        .args([
            "ls-remote",
            "--heads",
            target,
            &format!("refs/heads/{branch}"),
        ])
        .output()?;
    let sha = String::from_utf8_lossy(&remote.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_owned();
    println!("mirror: pushed {branch}");
    Ok(sha)
}

fn push_orphan_snapshot(
    root: &Path,
    included: &[PathBuf],
    keep_public: &[String],
    target: &str,
    branch: &str,
    tag: Option<&str>,
) -> Result<String> {
    let version = std::fs::read_to_string(root.join("VERSION"))?
        .trim()
        .to_owned();
    if let Some(tag) = tag {
        let existing_tag = Command::new("git")
            .args(["ls-remote", "--tags", target, &format!("refs/tags/{tag}")])
            .output()
            .context("cannot check remote tag")?;
        if !String::from_utf8_lossy(&existing_tag.stdout)
            .trim()
            .is_empty()
        {
            bail!("public tag `{tag}` already exists; refusing to move a release tag");
        }
    }

    let temp = tempfile::tempdir()?;
    let checkout = temp.path().join("target");
    let cloned = Command::new("git")
        .args(["clone", "--quiet", "--branch", branch, target])
        .arg(&checkout)
        .status()
        .context("cannot run git clone")?
        .success();
    let mut preserved = Vec::new();
    if cloned {
        for entry in fs::read_dir(&checkout)?.flatten() {
            if entry.file_name() == ".git" || !entry.path().is_file() {
                continue;
            }
            let rel = entry.file_name().to_string_lossy().to_string();
            if keep_matches(&rel, keep_public) {
                preserved.push((rel, fs::read(entry.path())?));
            }
        }
    }
    let snapshot_dir = temp.path().join("snapshot");
    fs::create_dir_all(&snapshot_dir)?;
    git(&snapshot_dir, &["init", "-q"])?;
    git(&snapshot_dir, &["checkout", "-q", "-B", branch])?;
    for relative in included {
        let destination = snapshot_dir.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(root.join(relative), &destination)?;
    }
    for (rel, bytes) in preserved {
        let path = snapshot_dir.join(&rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, bytes)?;
    }
    git(&snapshot_dir, &["add", "-A"])?;
    git(
        &snapshot_dir,
        &[
            "-c",
            "user.name=mdboard-mirror",
            "-c",
            "user.email=mirror@mdboard.io",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            &format!("Public snapshot {version}"),
        ],
    )?;
    if let Some(tag) = tag {
        git(
            &snapshot_dir,
            &["tag", "-a", tag, "-m", &format!("Release {version}")],
        )?;
    }
    git(&snapshot_dir, &["remote", "add", "origin", target])?;
    let old = Command::new("git")
        .args([
            "ls-remote",
            "--heads",
            target,
            &format!("refs/heads/{branch}"),
        ])
        .output()?;
    let old_sha = String::from_utf8_lossy(&old.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_owned();
    let refspec = format!("HEAD:{branch}");
    let lease = format!("--force-with-lease=refs/heads/{branch}:{old_sha}");
    let mut args = vec!["push", "--atomic", &lease, "origin", &refspec];
    let tagref;
    if let Some(tag) = tag {
        tagref = format!("refs/tags/{tag}");
        args.push(&tagref);
    }
    git(&snapshot_dir, &args)?;
    let remote = Command::new("git")
        .args([
            "ls-remote",
            "--heads",
            target,
            &format!("refs/heads/{branch}"),
        ])
        .output()?;
    let sha = String::from_utf8_lossy(&remote.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_owned();
    println!("mirror: pushed orphan snapshot {branch}");
    Ok(sha)
}

fn receipt_exists(root: &Path, profile: &str, target: &str) -> bool {
    let dir = root.join(".mdboard/mirror/receipts");
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let Ok(text) = fs::read_to_string(entry.path()) else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return false;
        };
        value.get("profile").and_then(|v| v.as_str()) == Some(profile)
            && value.get("target").and_then(|v| v.as_str()) == Some(target)
            && value
                .get("remote_commit")
                .and_then(|v| v.as_str())
                .is_some_and(|v| !v.is_empty())
    })
}

#[allow(clippy::too_many_arguments)]
fn write_receipt(
    root: &Path,
    profile: &str,
    target: &str,
    target_branch: &str,
    source_branch: &str,
    remote_commit: &str,
    tag: Option<&str>,
    authorised_by: &str,
    authorization: &str,
) -> Result<()> {
    let dir = root.join(".mdboard/mirror/receipts");
    fs::create_dir_all(&dir)?;
    let at = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let value = serde_json::json!({
        "schema": 1,
        "at": chrono::Utc::now().to_rfc3339(),
        "profile": profile,
        "source_commit": mdboard_git::rev_parse(root, "HEAD").unwrap_or_default(),
        "source_branch": source_branch,
        "target": target,
        "target_branch": target_branch,
        "snapshot": true,
        "tag": tag,
        "remote_commit": remote_commit,
        "checks": {
            "clean": true,
            "secret_scan": true,
            "target_reachable": true,
            "force_with_lease": true
        },
        "authorised_by": authorised_by,
        "authorization": authorization
    });
    fs::write(
        dir.join(format!("{at}-{profile}.json")),
        serde_json::to_string_pretty(&value)?,
    )?;
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .current_dir(root)
        .args(args)
        .status()
        .with_context(|| format!("cannot run git {}", args.join(" ")))?;
    if !status.success() {
        bail!("git {} failed", args.join(" "));
    }
    Ok(())
}
