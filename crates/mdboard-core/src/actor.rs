use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Whether an actor is a real human, an agent, or a service account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActorKind {
    Human,
    Agent,
    Service,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub vendor: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub kind: ActorKind,
    pub display_name: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub on_behalf_of: Option<String>,
    #[serde(default)]
    pub handles: BTreeMap<String, String>,
    #[serde(default)]
    pub git_emails: Vec<String>,
    #[serde(default)]
    pub git_names: Vec<String>,
    #[serde(default)]
    pub agent: Option<AgentInfo>,
    #[serde(default)]
    pub retired: bool,
    #[serde(default)]
    pub git_name: Option<String>,
    #[serde(default)]
    pub git_email: Option<String>,
}

impl Actor {
    #[must_use]
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|candidate| candidate == role)
    }
}

/// The optional `actors.toml` identity registry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actors {
    #[serde(default)]
    pub actors: BTreeMap<String, Actor>,
}

impl Actors {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn load_optional(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::default())
        }
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Actor> {
        self.actors.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::Actors;
    use std::io::Write;

    #[test]
    fn parses_actor_registry() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
[actors.marcus]
kind = "human"
display_name = "Marcus"
roles = ["owner"]
git_emails = ["marcus@example.com"]
"#
        )
        .unwrap();
        let actors = Actors::load(file.path()).unwrap();
        let actor = actors.get("marcus").unwrap();
        assert!(actor.has_role("owner"));
        assert_eq!(actor.git_emails.len(), 1);
    }
}
