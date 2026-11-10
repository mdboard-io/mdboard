//! Private-to-public snapshot mirroring.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use mdboard_driver::process::ProcessLoaded;
use serde::Deserialize;

const HARD_EXCLUDES: &[&str] = &[".git", ".mdboard", ".mdboardignore"];

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
    if !options.confirm_first_push {
        bail!("first public push requires manual review; rerun with --confirm-first-push");
    }
    push_snapshot(
        root,
        &included,
        &profile.keep_public,
        &target,
        &target_branch,
        options.force,
    )
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
        if HARD_EXCLUDES.contains(&name) || is_excluded(&rel, patterns) {
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

fn push_snapshot(
    root: &Path,
    included: &[PathBuf],
    keep_public: &[String],
    target: &str,
    branch: &str,
    force: bool,
) -> Result<()> {
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
        return Ok(());
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
    println!("mirror: pushed {branch}");
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
