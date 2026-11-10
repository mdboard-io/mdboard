//! Version parsing and comparison for mdboard project compatibility.
//!
//! mdboard versions are `MAJOR.MINOR.PATCH`. Leading zeroes are allowed so the product can use
//! the `1.0.00` display format while Cargo uses its semver-compatible `1.0.0`.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    raw: String,
}

impl Version {
    #[must_use]
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
            raw: format!("{major}.{minor}.{patch}"),
        }
    }

    #[must_use]
    pub const fn major(&self) -> u64 {
        self.major
    }

    #[must_use]
    pub const fn minor(&self) -> u64 {
        self.minor
    }

    #[must_use]
    pub const fn patch(&self) -> u64 {
        self.patch
    }

    #[must_use]
    pub fn is_breaking_from(&self, other: &Self) -> bool {
        self.major != other.major
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.raw)
    }
}

impl FromStr for Version {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let raw = value.trim();
        let parts: Vec<&str> = raw.split('.').collect();
        if parts.len() != 3 {
            return Err(format!(
                "invalid version `{raw}`; expected MAJOR.MINOR.PATCH"
            ));
        }
        let parse = |part: &str| {
            if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()) {
                return Err(format!("invalid version component `{part}`"));
            }
            part.parse::<u64>()
                .map_err(|_| format!("invalid version component `{part}`"))
        };
        Ok(Self {
            major: parse(parts[0])?,
            minor: parse(parts[1])?,
            patch: parse(parts[2])?,
            raw: raw.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Version;

    #[test]
    fn parses_zero_padded_patch_versions() {
        let version: Version = "1.0.00".parse().unwrap();
        assert_eq!(version.major(), 1);
        assert_eq!(version.minor(), 0);
        assert_eq!(version.patch(), 0);
        assert_eq!(version.to_string(), "1.0.00");
    }

    #[test]
    fn compares_numerically_not_lexically() {
        let older: Version = "1.0.00".parse().unwrap();
        let newer: Version = "1.0.01".parse().unwrap();
        assert!(newer > older);
        assert_eq!(older.major(), newer.major());
    }

    #[test]
    fn rejects_malformed_versions() {
        assert!("1.0".parse::<Version>().is_err());
        assert!("1.0.x".parse::<Version>().is_err());
        assert!("1.0.00.1".parse::<Version>().is_err());
    }
}
