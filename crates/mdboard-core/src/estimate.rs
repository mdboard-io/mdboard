//! PERT effort estimates shared by batches and tasks.

use serde::{Deserialize, Serialize};

/// An optimistic/realistic/pessimistic estimate with its weighted value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffortEstimate {
    pub optimistic: f64,
    pub realistic: f64,
    pub pessimistic: f64,
    pub weighted: f64,
}

impl EffortEstimate {
    #[must_use]
    pub fn from_parts(optimistic: f64, realistic: f64, pessimistic: f64) -> Self {
        let weighted = (optimistic + 4.0 * realistic + pessimistic) / 6.0;
        Self {
            optimistic,
            realistic,
            pessimistic,
            weighted,
        }
    }

    pub fn recompute(&mut self) {
        self.weighted = (self.optimistic + 4.0 * self.realistic + self.pessimistic) / 6.0;
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        [
            self.optimistic,
            self.realistic,
            self.pessimistic,
            self.weighted,
        ]
        .iter()
        .all(|value| value.is_finite() && *value >= 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::EffortEstimate;

    #[test]
    fn pert_weighting_and_validation() {
        let estimate = EffortEstimate::from_parts(8_000.0, 12_000.0, 20_000.0);
        assert!((estimate.weighted - 12_666.666_666_666_666).abs() < 0.001);
        assert!(estimate.is_valid());
        assert!(!EffortEstimate::from_parts(-1.0, 1.0, 1.0).is_valid());
    }
}
