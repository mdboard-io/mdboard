//! Built-in version, tag and build metadata management.

use std::fmt;
use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::{NaiveDate, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        let mut parts = value.split('.');
        let (Some(major), Some(minor), Some(patch), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            bail!("invalid version `{value}`; expected MAJOR.MINOR.PATCH");
        };
        if major.starts_with('0') && major.len() > 1
            || minor.starts_with('0') && minor.len() > 1
            || patch.starts_with('0') && patch.len() > 1
        {
            bail!("version `{value}` uses zero-padding; strict SemVer is required");
        }
        Ok(Self {
            major: major.parse().context("major version")?,
            minor: minor.parse().context("minor version")?,
            patch: patch.parse().context("patch version")?,
        })
    }

    #[must_use]
    pub fn bump(self, part: Part) -> Self {
        match part {
            Part::Major => Self {
                major: self.major + 1,
                minor: 0,
                patch: 0,
            },
            Part::Minor => Self {
                minor: self.minor + 1,
                patch: 0,
                ..self
            },
            Part::Patch => Self {
                patch: self.patch + 1,
                ..self
            },
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Part {
    Major,
    Minor,
    Patch,
}

pub fn branch_tag(branch: &str) -> &str {
    match branch {
        "upstream" => "preview",
        "nightly" => "nightly",
        "stable" => "stable",
        _ => "dev",
    }
}

pub fn read_version(root: &Path) -> Result<Version> {
    let text = std::fs::read_to_string(root.join("VERSION"))?;
    Version::parse(&text)
}

pub fn write_version(root: &Path, version: Version) -> Result<()> {
    atomic_write(&root.join("VERSION"), &format!("{version}\n"))?;
    sync_cargo(root, version)
}

fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temp, content)?;
    std::fs::rename(temp, path)?;
    Ok(())
}

fn sync_cargo(root: &Path, version: Version) -> Result<()> {
    let cargo_path = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&cargo_path)?;
    let value = version.to_string();
    let mut in_workspace = false;
    let mut replaced = false;
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    for line in &mut lines {
        if line.trim() == "[workspace.package]" {
            in_workspace = true;
        } else if line.starts_with('[') {
            in_workspace = false;
        } else if in_workspace && line.trim_start().starts_with("version =") {
            *line = format!("version = \"{value}\"");
            replaced = true;
        }
    }
    if !replaced {
        bail!("cannot find [workspace.package] version in Cargo.toml");
    }
    std::fs::write(&cargo_path, lines.join("\n") + "\n")?;

    let lock_path = root.join("Cargo.lock");
    if lock_path.is_file() {
        let lock = std::fs::read_to_string(&lock_path)?;
        let mut in_package = false;
        let mut mdboard_package = false;
        let mut lines: Vec<String> = lock.lines().map(str::to_owned).collect();
        for line in &mut lines {
            if line == "[[package]]" {
                in_package = true;
                mdboard_package = false;
            } else if in_package && line.starts_with("name = \"mdboard") {
                mdboard_package = true;
            } else if in_package && mdboard_package && line.starts_with("version = ") {
                *line = format!("version = \"{value}\"");
            } else if line.starts_with("[[") && line != "[[package]]" {
                in_package = false;
            }
        }
        std::fs::write(&lock_path, lines.join("\n") + "\n")?;
    }
    Ok(())
}

pub fn set_version(root: &Path, value: &str) -> Result<Version> {
    let version = Version::parse(value)?;
    write_version(root, version)?;
    Ok(version)
}

pub fn bump(root: &Path, part: Part) -> Result<Version> {
    let version = read_version(root)?.bump(part);
    write_version(root, version)?;
    Ok(version)
}

pub fn close_iteration(root: &Path) -> Result<Version> {
    let version = read_version(root)?.bump(Part::Minor);
    write_version(root, version)?;
    Ok(version)
}

pub fn read_tag(root: &Path) -> Result<String> {
    Ok(std::fs::read_to_string(root.join("TAG"))?.trim().to_owned())
}

pub fn write_tag(root: &Path, tag: &str) -> Result<()> {
    let allowed = tag.is_empty()
        || tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !allowed {
        bail!("invalid tag `{tag}`");
    }
    atomic_write(&root.join("TAG"), &format!("{tag}\n"))
}

pub fn sync_tag(root: &Path, branch: &str) -> Result<String> {
    let tag = branch_tag(branch).to_owned();
    write_tag(root, &tag)?;
    Ok(tag)
}

pub fn prepare_build(root: &Path, branch: &str) -> Result<String> {
    sync_tag(root, branch)?;
    let today = Utc::now().format("%Y%m%d").to_string();
    let previous = std::fs::read_to_string(root.join("BUILD"))?
        .trim()
        .to_owned();
    let (previous_date, previous_counter) = previous
        .split_once('-')
        .with_context(|| format!("invalid BUILD `{previous}`"))?;
    let counter = if previous_date == today {
        previous_counter.parse::<u32>().context("BUILD counter")? + 1
    } else {
        0
    };
    let build = format!("{today}-{counter}");
    atomic_write(&root.join("BUILD"), &format!("{build}\n"))?;
    let date = Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
    atomic_write(&root.join("BUILD_DATE"), &format!("{date}\n"))?;
    Ok(build)
}

pub fn check_fresh(root: &Path, branch: &str) -> Result<()> {
    let version = read_version(root)?;
    let expected_tag = branch_tag(branch);
    let tag = read_tag(root)?;
    if tag != expected_tag {
        bail!("TAG is `{tag}`, expected `{expected_tag}` for branch `{branch}`");
    }
    let build = std::fs::read_to_string(root.join("BUILD"))?
        .trim()
        .to_owned();
    let (date, counter) = build
        .split_once('-')
        .with_context(|| format!("invalid BUILD `{build}`"))?;
    if date != Utc::now().format("%Y%m%d").to_string() || counter.parse::<u32>().is_err() {
        bail!("BUILD `{build}` is stale or malformed");
    }
    let build_date = std::fs::read_to_string(root.join("BUILD_DATE"))?
        .trim()
        .to_owned();
    NaiveDate::parse_from_str(build_date.get(..10).unwrap_or(""), "%Y-%m-%d")
        .context("invalid BUILD_DATE")?;
    let cargo = std::fs::read_to_string(root.join("Cargo.toml"))?;
    if !cargo.contains(&format!("version = \"{version}\"")) {
        bail!("Cargo.toml version is not synchronized with VERSION");
    }
    Ok(())
}

pub fn show(root: &Path) -> Result<()> {
    println!("version: {}", read_version(root)?);
    println!("tag: {}", read_tag(root)?);
    println!(
        "build: {}",
        std::fs::read_to_string(root.join("BUILD"))?.trim()
    );
    println!(
        "build_date: {}",
        std::fs::read_to_string(root.join("BUILD_DATE"))?.trim()
    );
    Ok(())
}

pub fn write_release(root: &Path) -> Result<String> {
    let version = read_version(root)?;
    let tag = read_tag(root)?;
    let content = format!(
        "# Generated by mdboard. Do not edit by hand.\nversion = \"{version}\"\ntag = \"{tag}\"\n"
    );
    atomic_write(&root.join("RELEASE"), &content)?;
    Ok(format!(
        "RELEASE: {version} (tag {})",
        if tag.is_empty() { "release" } else { &tag }
    ))
}

#[cfg(test)]
mod tests {
    use super::{Part, Version, branch_tag};

    #[test]
    fn rejects_zero_padded_versions() {
        assert!(Version::parse("0.1990.06").is_err());
    }

    #[test]
    fn bumps_reset_lower_fields() {
        let version = Version::parse("1.2.3").unwrap();
        assert_eq!(version.bump(Part::Patch).to_string(), "1.2.4");
        assert_eq!(version.bump(Part::Minor).to_string(), "1.3.0");
        assert_eq!(version.bump(Part::Major).to_string(), "2.0.0");
    }

    #[test]
    fn branch_tags_are_lifecycle_aware() {
        assert_eq!(branch_tag("upstream"), "preview");
        assert_eq!(branch_tag("nightly"), "nightly");
        assert_eq!(branch_tag("stable"), "stable");
        assert_eq!(branch_tag("feature"), "dev");
    }
}
