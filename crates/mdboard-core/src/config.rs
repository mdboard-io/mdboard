//! Cost and plugin configuration.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const DEFAULT_SUBSCRIPTION: &str = "unspecified payment model";
pub const DEFAULT_EUR_PER_MILLION_TOKENS: f64 = 4.96;

/// The blended token cost model used to turn estimates into EUR.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostConfig {
    #[serde(default = "default_subscription")]
    pub subscription: String,
    #[serde(default = "default_eur_per_million_tokens")]
    pub eur_per_million_tokens: f64,
}

fn default_subscription() -> String {
    DEFAULT_SUBSCRIPTION.to_owned()
}

const fn default_eur_per_million_tokens() -> f64 {
    DEFAULT_EUR_PER_MILLION_TOKENS
}

impl Default for CostConfig {
    fn default() -> Self {
        Self {
            subscription: default_subscription(),
            eur_per_million_tokens: default_eur_per_million_tokens(),
        }
    }
}

impl CostConfig {
    #[must_use]
    pub fn estimated_cost_eur(&self, tokens: f64) -> f64 {
        let cost = tokens * self.eur_per_million_tokens / 1_000_000.0;
        if cost == 0.0 { 0.0 } else { cost }
    }
}

#[cfg(test)]
mod tests {
    use super::CostConfig;

    #[test]
    fn computes_cost_and_defaults() {
        let costs = CostConfig::default();
        assert_eq!(costs.subscription, "unspecified payment model");
        assert!((costs.estimated_cost_eur(1_000_000.0) - 4.96).abs() < 0.001);
        assert_eq!(CostConfig::default().estimated_cost_eur(0.0), 0.0);
    }
}

/// An owner-approved external plugin manifest. Execution is not enabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginConfig {
    pub kind: String,
    pub command: PathBuf,
    pub version: String,
    pub sha256: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub network: bool,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

const fn default_timeout() -> u64 {
    30
}

pub type Plugins = BTreeMap<String, PluginConfig>;
