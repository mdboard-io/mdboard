#![allow(dead_code)]

//! Net migration summaries.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Changed,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationChange {
    pub key: String,
    pub kind: ChangeKind,
    pub title: String,
    pub description: String,
    pub impact: String,
    pub action: String,
}

#[derive(Debug, Clone)]
pub struct MigrationStep {
    pub from: String,
    pub to: String,
    pub installed_version: String,
    pub installed_build_date: String,
    pub target_version: String,
    pub target_build_date: String,
    pub changes: Vec<MigrationChange>,
}

#[derive(Debug, Clone, Default)]
pub struct NetSummary {
    pub from: String,
    pub to: String,
    pub installed_version: String,
    pub installed_build_date: String,
    pub target_version: String,
    pub target_build_date: String,
    pub changes: Vec<MigrationChange>,
}

impl NetSummary {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    #[must_use]
    pub fn render(&self) -> String {
        let mut out = format!(
            "Migration {} → {}\ninstalled: {} ({})\ntarget: {} ({})\n",
            self.from,
            self.to,
            self.installed_version,
            self.installed_build_date,
            self.target_version,
            self.target_build_date
        );
        for kind in [ChangeKind::Added, ChangeKind::Changed, ChangeKind::Removed] {
            let label = match kind {
                ChangeKind::Added => "New",
                ChangeKind::Changed => "Changed",
                ChangeKind::Removed => "Removed",
            };
            let rows: Vec<_> = self
                .changes
                .iter()
                .filter(|change| change.kind == kind)
                .collect();
            if rows.is_empty() {
                continue;
            }
            out.push_str(&format!("\n{label}:\n"));
            for change in rows {
                out.push_str(&format!("- {}: {}\n", change.title, change.description));
                if !change.impact.is_empty() {
                    out.push_str(&format!("  Impact: {}\n", change.impact));
                }
                if !change.action.is_empty() {
                    out.push_str(&format!("  Action: {}\n", change.action));
                }
            }
        }
        out
    }
}

#[must_use]
pub fn aggregate(steps: &[MigrationStep]) -> NetSummary {
    let mut changes = BTreeMap::new();
    for step in steps {
        for change in &step.changes {
            match change.kind {
                ChangeKind::Added => {
                    changes.insert(change.key.clone(), change.clone());
                }
                ChangeKind::Changed => {
                    if let Some(existing) = changes.get_mut(&change.key) {
                        if existing.kind != ChangeKind::Removed {
                            *existing = change.clone();
                        }
                    } else {
                        changes.insert(change.key.clone(), change.clone());
                    }
                }
                ChangeKind::Removed => {
                    if changes
                        .get(&change.key)
                        .is_some_and(|existing| existing.kind == ChangeKind::Added)
                    {
                        changes.remove(&change.key);
                    } else {
                        changes.insert(change.key.clone(), change.clone());
                    }
                }
            }
        }
    }
    let first = steps.first();
    let last = steps.last();
    NetSummary {
        from: first.map_or_else(String::new, |step| step.from.clone()),
        to: last.map_or_else(String::new, |step| step.to.clone()),
        installed_version: first.map_or_else(String::new, |step| step.installed_version.clone()),
        installed_build_date: first
            .map_or_else(String::new, |step| step.installed_build_date.clone()),
        target_version: last.map_or_else(String::new, |step| step.target_version.clone()),
        target_build_date: last.map_or_else(String::new, |step| step.target_build_date.clone()),
        changes: changes.into_values().collect(),
    }
}

#[must_use]
pub fn schema_summary(from: u32, to: u32, version: &str, build_date: &str) -> NetSummary {
    let changes = if from == 2 && to == 3 {
        vec![
            MigrationChange {
                key: "governance-config".to_owned(),
                kind: ChangeKind::Added,
                title: "Governance configuration".to_owned(),
                description:
                    "Stakeholder seats, quorum, guardian and co-guardians are configurable."
                        .to_owned(),
                impact: "Boards can record stakeholder votes before acceptance.".to_owned(),
                action: "Review [stakeholders] and [guardian], then add seats if needed."
                    .to_owned(),
            },
            MigrationChange {
                key: "votes".to_owned(),
                kind: ChangeKind::Added,
                title: "Stakeholder votes".to_owned(),
                description: "Vote artifacts and the accepted gate are available.".to_owned(),
                impact: "A pitched card may require a passed stakeholder vote.".to_owned(),
                action: "No action when there are no stakeholder seats.".to_owned(),
            },
        ]
    } else {
        Vec::new()
    };
    NetSummary {
        from: from.to_string(),
        to: to.to_string(),
        installed_version: version.to_owned(),
        installed_build_date: build_date.to_owned(),
        target_version: version.to_owned(),
        target_build_date: build_date.to_owned(),
        changes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(key: &str, kind: ChangeKind) -> MigrationChange {
        MigrationChange {
            key: key.to_owned(),
            kind,
            title: key.to_owned(),
            description: format!("{kind:?} {key}"),
            impact: String::new(),
            action: String::new(),
        }
    }

    fn step(from: &str, to: &str, changes: Vec<MigrationChange>) -> MigrationStep {
        MigrationStep {
            from: from.to_owned(),
            to: to.to_owned(),
            installed_version: from.to_owned(),
            installed_build_date: "old".to_owned(),
            target_version: to.to_owned(),
            target_build_date: "new".to_owned(),
            changes,
        }
    }

    #[test]
    fn introduced_then_removed_is_cancelled() {
        let summary = aggregate(&[
            step("1", "2", vec![change("x", ChangeKind::Added)]),
            step("2", "3", vec![change("x", ChangeKind::Removed)]),
        ]);
        assert!(summary.is_empty());
    }

    #[test]
    fn repeated_change_uses_final_state() {
        let mut second = change("x", ChangeKind::Changed);
        second.title = "final".to_owned();
        let summary = aggregate(&[
            step("1", "2", vec![change("x", ChangeKind::Changed)]),
            step("2", "3", vec![second]),
        ]);
        assert_eq!(summary.changes[0].title, "final");
    }
}
