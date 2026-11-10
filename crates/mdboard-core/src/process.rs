//! The 1.0 process model: ideas, batches, tasks, reviews, meetings,
//! upstream iterations and the append-only event journal.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::config::CostConfig;
use crate::error::{Error, Result};
use crate::estimate::EffortEstimate;

/// The kind of card. A card's kind determines which states and actions apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[serde(alias = "idea")]
    Proposal,
    Defect,
    Research,
    Batch,
    Task,
}

impl Kind {
    pub const ALL: [Self; 5] = [
        Self::Proposal,
        Self::Defect,
        Self::Research,
        Self::Batch,
        Self::Task,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Proposal => "proposal",
            Self::Defect => "defect",
            Self::Research => "research",
            Self::Batch => "batch",
            Self::Task => "task",
        }
    }

    #[must_use]
    pub const fn is_intent(self) -> bool {
        matches!(self, Self::Proposal | Self::Defect | Self::Research)
    }

    #[must_use]
    pub const fn states(self) -> &'static [State] {
        match self {
            Self::Proposal | Self::Defect | Self::Research => &INTENT_STATES,
            Self::Batch => &BATCH_STATES,
            Self::Task => &TASK_STATES,
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Kind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value.trim())
            .ok_or_else(|| Error::Message(format!("unknown card kind `{value}`")))
    }
}

/// The lifecycle state of a card.
///
/// `Canceled` is shared by all kinds. `Blocked` is batch-only; `Error` is
/// task-only. The kind/state validity rule lives in [`State::allowed_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    #[serde(alias = "spark")]
    Draft,
    Refine,
    Pitch,
    Rejected,
    Accepted,
    Roadmap,
    Breakdown,
    Doing,
    Handover,
    Completed,
    Blocked,
    Pending,
    Working,
    Done,
    Error,
    Obsolete,
    Canceled,
}

pub const INTENT_STATES: [State; 5] = [
    State::Draft,
    State::Refine,
    State::Pitch,
    State::Rejected,
    State::Canceled,
];

pub const BATCH_STATES: [State; 8] = [
    State::Accepted,
    State::Roadmap,
    State::Breakdown,
    State::Doing,
    State::Handover,
    State::Completed,
    State::Blocked,
    State::Canceled,
];

pub const TASK_STATES: [State; 6] = [
    State::Pending,
    State::Working,
    State::Done,
    State::Error,
    State::Obsolete,
    State::Canceled,
];

impl State {
    pub const ALL: [Self; 17] = [
        Self::Draft,
        Self::Refine,
        Self::Pitch,
        Self::Rejected,
        Self::Accepted,
        Self::Roadmap,
        Self::Breakdown,
        Self::Doing,
        Self::Handover,
        Self::Completed,
        Self::Blocked,
        Self::Pending,
        Self::Working,
        Self::Done,
        Self::Error,
        Self::Obsolete,
        Self::Canceled,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Refine => "refine",
            Self::Pitch => "pitch",
            Self::Rejected => "rejected",
            Self::Accepted => "accepted",
            Self::Roadmap => "roadmap",
            Self::Breakdown => "breakdown",
            Self::Doing => "doing",
            Self::Handover => "handover",
            Self::Completed => "completed",
            Self::Blocked => "blocked",
            Self::Pending => "pending",
            Self::Working => "working",
            Self::Done => "done",
            Self::Error => "error",
            Self::Obsolete => "obsolete",
            Self::Canceled => "canceled",
        }
    }

    #[must_use]
    pub const fn allowed_for(self, kind: Kind) -> bool {
        match self {
            Self::Draft | Self::Refine | Self::Pitch | Self::Rejected => kind.is_intent(),
            Self::Accepted
            | Self::Roadmap
            | Self::Breakdown
            | Self::Doing
            | Self::Handover
            | Self::Completed
            | Self::Blocked => matches!(kind, Kind::Batch),
            Self::Pending | Self::Working | Self::Done | Self::Error | Self::Obsolete => {
                matches!(kind, Kind::Task)
            }
            Self::Canceled => true,
        }
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Rejected | Self::Completed | Self::Done | Self::Obsolete | Self::Canceled
        )
    }

    #[must_use]
    pub const fn is_batch_active(self) -> bool {
        matches!(self, Self::Breakdown | Self::Doing | Self::Handover)
    }

    #[must_use]
    pub const fn is_task_active(self) -> bool {
        matches!(self, Self::Working)
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for State {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|state| state.as_str() == value.trim())
            .ok_or_else(|| Error::Message(format!("unknown state `{value}`")))
    }
}

/// A named process action. Named actions replace the old generic `card move`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Draft,
    Refine,
    Pitch,
    AcceptIdea,
    RejectIdea,
    Park,
    Reopen,
    RequestChanges,
    Estimate,
    Reestimate,
    Roadmap,
    Unroadmap,
    Breakdown,
    AbortBreakdown,
    Start,
    Handover,
    Complete,
    Work,
    Done,
    Fail,
    Recover,
    Obsolete,
    ReopenTask,
    Block,
    Unblock,
    Cancel,
    Review,
    Merge,
    Release,
}

impl Action {
    pub const ALL: [Self; 29] = [
        Self::Draft,
        Self::Refine,
        Self::Pitch,
        Self::AcceptIdea,
        Self::RejectIdea,
        Self::Park,
        Self::Reopen,
        Self::RequestChanges,
        Self::Estimate,
        Self::Reestimate,
        Self::Roadmap,
        Self::Unroadmap,
        Self::Breakdown,
        Self::AbortBreakdown,
        Self::Start,
        Self::Handover,
        Self::Complete,
        Self::Work,
        Self::Done,
        Self::Fail,
        Self::Recover,
        Self::Obsolete,
        Self::ReopenTask,
        Self::Block,
        Self::Unblock,
        Self::Cancel,
        Self::Review,
        Self::Merge,
        Self::Release,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Refine => "refine",
            Self::Pitch => "pitch",
            Self::AcceptIdea => "accept-idea",
            Self::RejectIdea => "reject-idea",
            Self::Park => "park",
            Self::Reopen => "reopen",
            Self::RequestChanges => "request-changes",
            Self::Estimate => "estimate",
            Self::Reestimate => "reestimate",
            Self::Roadmap => "roadmap",
            Self::Unroadmap => "unroadmap",
            Self::Breakdown => "breakdown",
            Self::AbortBreakdown => "abort-breakdown",
            Self::Start => "start",
            Self::Handover => "handover",
            Self::Complete => "complete",
            Self::Work => "work",
            Self::Done => "done",
            Self::Fail => "fail",
            Self::Recover => "recover",
            Self::Obsolete => "obsolete",
            Self::ReopenTask => "reopen-task",
            Self::Block => "block",
            Self::Unblock => "unblock",
            Self::Cancel => "cancel",
            Self::Review => "review",
            Self::Merge => "merge",
            Self::Release => "release",
        }
    }

    #[must_use]
    pub const fn is_state_transition(self) -> bool {
        matches!(
            self,
            Self::Refine
                | Self::Pitch
                | Self::AcceptIdea
                | Self::RejectIdea
                | Self::Park
                | Self::Reopen
                | Self::RequestChanges
                | Self::Roadmap
                | Self::Unroadmap
                | Self::Breakdown
                | Self::AbortBreakdown
                | Self::Start
                | Self::Handover
                | Self::Complete
                | Self::Work
                | Self::Done
                | Self::Fail
                | Self::Recover
                | Self::Obsolete
                | Self::ReopenTask
                | Self::Block
                | Self::Unblock
                | Self::Cancel
        )
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Action {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|action| action.as_str() == value.trim())
            .ok_or_else(|| Error::Message(format!("unknown action `{value}`")))
    }
}

/// A human or agent role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Owner,
    Member,
    Agent,
}

impl Role {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Member => "member",
            Self::Agent => "agent",
        }
    }
}

/// The authority required for an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Authority {
    /// The agent may act alone.
    Agent,
    /// Any human may authorise; the agent may execute with recorded authorisation.
    Human,
    /// The owner must authorise; the agent may execute with recorded authorisation.
    Owner,
    /// The owner must execute; the agent only prepares.
    OwnerExecuted,
}

impl Authority {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Human => "human",
            Self::Owner => "owner",
            Self::OwnerExecuted => "owner-executed",
        }
    }

    #[must_use]
    pub const fn allows(self, role: Role) -> bool {
        match self {
            Self::Agent => matches!(role, Role::Agent),
            Self::Human => matches!(role, Role::Owner | Role::Member),
            Self::Owner | Self::OwnerExecuted => matches!(role, Role::Owner),
        }
    }

    #[must_use]
    pub const fn agent_may_execute(self) -> bool {
        !matches!(self, Self::OwnerExecuted)
    }
}

/// One row in the transition table.
#[derive(Debug, Clone, Copy)]
pub struct Transition {
    pub action: Action,
    pub from: State,
    pub to: State,
    pub from_kind: Kind,
    pub to_kind: Option<Kind>,
    pub authority: Authority,
    pub guard: &'static str,
}

const fn t(
    action: Action,
    from: State,
    to: State,
    from_kind: Kind,
    authority: Authority,
    guard: &'static str,
) -> Transition {
    Transition {
        action,
        from,
        to,
        from_kind,
        to_kind: None,
        authority,
        guard,
    }
}

/// The complete transition table. `Unblock` is dynamic (it returns to the
/// recorded blocked state) and is therefore handled separately.
pub static TRANSITIONS: &[Transition] = &[
    // Idea machine.
    t(
        Action::Refine,
        State::Draft,
        State::Refine,
        Kind::Proposal,
        Authority::Human,
        "card is a spark",
    ),
    t(
        Action::Pitch,
        State::Refine,
        State::Pitch,
        Kind::Proposal,
        Authority::Human,
        "content is ready for the consortium",
    ),
    t(
        Action::Park,
        State::Refine,
        State::Draft,
        Kind::Proposal,
        Authority::Human,
        "creator parks the idea",
    ),
    Transition {
        action: Action::AcceptIdea,
        from: State::Pitch,
        to: State::Accepted,
        from_kind: Kind::Proposal,
        to_kind: Some(Kind::Batch),
        authority: Authority::Human,
        guard: "consortium decision is recorded",
    },
    t(
        Action::RejectIdea,
        State::Pitch,
        State::Rejected,
        Kind::Proposal,
        Authority::Human,
        "consortium rejects the idea",
    ),
    t(
        Action::RequestChanges,
        State::Pitch,
        State::Refine,
        Kind::Proposal,
        Authority::Human,
        "consortium requests changes",
    ),
    t(
        Action::Reopen,
        State::Rejected,
        State::Draft,
        Kind::Proposal,
        Authority::Human,
        "owner or creator reopens the idea",
    ),
    t(
        Action::Cancel,
        State::Draft,
        State::Canceled,
        Kind::Proposal,
        Authority::Human,
        "reason is provided",
    ),
    t(
        Action::Cancel,
        State::Refine,
        State::Canceled,
        Kind::Proposal,
        Authority::Human,
        "reason is provided",
    ),
    t(
        Action::Cancel,
        State::Pitch,
        State::Canceled,
        Kind::Proposal,
        Authority::Human,
        "reason is provided",
    ),
    // Batch machine.
    t(
        Action::Roadmap,
        State::Accepted,
        State::Roadmap,
        Kind::Batch,
        Authority::Human,
        "estimate present; roadmap limit permits",
    ),
    t(
        Action::Unroadmap,
        State::Roadmap,
        State::Accepted,
        Kind::Batch,
        Authority::Human,
        "no non-canceled tasks",
    ),
    t(
        Action::Breakdown,
        State::Roadmap,
        State::Breakdown,
        Kind::Batch,
        Authority::Human,
        "estimate present; no other active batch",
    ),
    t(
        Action::AbortBreakdown,
        State::Breakdown,
        State::Roadmap,
        Kind::Batch,
        Authority::Human,
        "pending tasks are canceled",
    ),
    t(
        Action::Start,
        State::Breakdown,
        State::Doing,
        Kind::Batch,
        Authority::Human,
        "clarification accepted; tasks pending",
    ),
    t(
        Action::Handover,
        State::Doing,
        State::Handover,
        Kind::Batch,
        Authority::Agent,
        "all tasks done; no task error",
    ),
    t(
        Action::RequestChanges,
        State::Handover,
        State::Doing,
        Kind::Batch,
        Authority::Human,
        "changes-required review artifact exists",
    ),
    t(
        Action::Complete,
        State::Handover,
        State::Completed,
        Kind::Batch,
        Authority::Owner,
        "independent review, owner accept, report and changelog",
    ),
    t(
        Action::Block,
        State::Accepted,
        State::Blocked,
        Kind::Batch,
        Authority::Human,
        "reason provided",
    ),
    t(
        Action::Block,
        State::Roadmap,
        State::Blocked,
        Kind::Batch,
        Authority::Human,
        "reason provided",
    ),
    t(
        Action::Block,
        State::Breakdown,
        State::Blocked,
        Kind::Batch,
        Authority::Human,
        "reason provided",
    ),
    t(
        Action::Obsolete,
        State::Pending,
        State::Obsolete,
        Kind::Task,
        Authority::Agent,
        "task is duplicate or no longer needed",
    ),
    t(
        Action::Obsolete,
        State::Working,
        State::Obsolete,
        Kind::Task,
        Authority::Agent,
        "stop obsolete task",
    ),
    t(
        Action::Obsolete,
        State::Error,
        State::Obsolete,
        Kind::Task,
        Authority::Agent,
        "task is obsolete",
    ),
    t(
        Action::ReopenTask,
        State::Obsolete,
        State::Pending,
        Kind::Task,
        Authority::Agent,
        "task is needed again",
    ),
    t(
        Action::Block,
        State::Doing,
        State::Blocked,
        Kind::Batch,
        Authority::Human,
        "reason provided",
    ),
    t(
        Action::Block,
        State::Handover,
        State::Blocked,
        Kind::Batch,
        Authority::Human,
        "reason provided",
    ),
    t(
        Action::Cancel,
        State::Accepted,
        State::Canceled,
        Kind::Batch,
        Authority::Human,
        "reason provided",
    ),
    t(
        Action::Cancel,
        State::Roadmap,
        State::Canceled,
        Kind::Batch,
        Authority::Human,
        "reason provided",
    ),
    t(
        Action::Cancel,
        State::Breakdown,
        State::Canceled,
        Kind::Batch,
        Authority::Human,
        "reason provided",
    ),
    t(
        Action::Cancel,
        State::Doing,
        State::Canceled,
        Kind::Batch,
        Authority::Human,
        "reason provided",
    ),
    t(
        Action::Cancel,
        State::Handover,
        State::Canceled,
        Kind::Batch,
        Authority::Human,
        "reason provided",
    ),
    t(
        Action::Cancel,
        State::Blocked,
        State::Canceled,
        Kind::Batch,
        Authority::Human,
        "reason provided",
    ),
    // Task machine.
    t(
        Action::Work,
        State::Pending,
        State::Working,
        Kind::Task,
        Authority::Agent,
        "batch doing; no other task working",
    ),
    t(
        Action::Done,
        State::Working,
        State::Done,
        Kind::Task,
        Authority::Agent,
        "commit and evidence recorded",
    ),
    t(
        Action::Fail,
        State::Working,
        State::Error,
        Kind::Task,
        Authority::Agent,
        "reason provided",
    ),
    t(
        Action::Recover,
        State::Error,
        State::Pending,
        Kind::Task,
        Authority::Human,
        "human recovery instruction",
    ),
    t(
        Action::Cancel,
        State::Pending,
        State::Canceled,
        Kind::Task,
        Authority::Human,
        "reason provided",
    ),
    t(
        Action::Cancel,
        State::Error,
        State::Canceled,
        Kind::Task,
        Authority::Human,
        "reason provided",
    ),
];

/// Find the transition for a kind, state and action.
#[must_use]
pub fn find_transition(kind: Kind, from: State, action: Action) -> Option<&'static Transition> {
    TRANSITIONS.iter().find(|rule| {
        (rule.from_kind == kind || (rule.from_kind == Kind::Proposal && kind.is_intent()))
            && rule.from == from
            && rule.action == action
    })
}

/// The effort estimate carried by a batch (and optionally a task).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Estimate {
    pub tokens: EffortEstimate,
    pub agent_hours: EffortEstimate,
}

impl Estimate {
    #[must_use]
    pub fn from_parts(tokens: (f64, f64, f64), agent_hours: (f64, f64, f64)) -> Self {
        Self {
            tokens: EffortEstimate::from_parts(tokens.0, tokens.1, tokens.2),
            agent_hours: EffortEstimate::from_parts(agent_hours.0, agent_hours.1, agent_hours.2),
        }
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.tokens.is_valid() && self.agent_hours.is_valid()
    }
}

/// A card in the 1.0 model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessCard {
    pub id: String,
    pub kind: Kind,
    pub state: State,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub created_by: String,
    pub intent_owner: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub origin_kind: Option<Kind>,
    #[serde(default)]
    pub source_review: Option<String>,
    #[serde(default)]
    pub estimate: Option<Estimate>,
    #[serde(default)]
    pub report: Option<String>,
    #[serde(default)]
    pub blocked_from: Option<State>,
    #[serde(default)]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub archived_at: Option<String>,
    #[serde(default)]
    pub archived_by: Option<String>,
    #[serde(default)]
    pub archive_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl ProcessCard {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Self::parse(path, &text)
    }

    pub fn parse(path: &std::path::Path, text: &str) -> Result<Self> {
        let (yaml, body) = split_front_matter(path, text)?;
        let mut card: Self = serde_yaml::from_str(&yaml)
            .map_err(|error| Error::card(path, format!("invalid front matter: {error}")))?;
        card.body = body.trim().to_owned();
        card.validate()?;
        Ok(card)
    }

    pub fn to_markdown(&self) -> Result<String> {
        let mut value = serde_yaml::to_value(self)?;
        if let Some(table) = value.as_mapping_mut() {
            table.remove(serde_yaml::Value::String("body".to_owned()));
        }
        let yaml = serde_yaml::to_string(&value)?;
        Ok(format!("---\n{}---\n\n{}\n", yaml, self.body.trim()))
    }

    #[must_use]
    pub fn new_idea(
        id: impl Into<String>,
        title: impl Into<String>,
        intent_owner: impl Into<String>,
        at: impl Into<String>,
    ) -> Self {
        let at = at.into();
        let intent_owner = intent_owner.into();
        Self {
            id: id.into(),
            kind: Kind::Proposal,
            state: State::Draft,
            title: title.into(),
            body: "## Intent\n".to_owned(),
            created_by: intent_owner.clone(),
            intent_owner,
            parent: None,
            depends_on: Vec::new(),
            origin_kind: None,
            source_review: None,
            estimate: None,
            report: None,
            blocked_from: None,
            blocked_reason: None,
            archived_at: None,
            archived_by: None,
            archive_reason: None,
            created_at: at.clone(),
            updated_at: at,
        }
    }

    #[must_use]
    pub fn new_intent(
        kind: Kind,
        id: impl Into<String>,
        title: impl Into<String>,
        intent_owner: impl Into<String>,
        at: impl Into<String>,
    ) -> Self {
        assert!(kind.is_intent(), "new_intent requires an intent kind");
        let at = at.into();
        let intent_owner = intent_owner.into();
        Self {
            id: id.into(),
            kind,
            state: State::Draft,
            title: title.into(),
            body: "## Intent\n".to_owned(),
            created_by: "agent".to_owned(),
            intent_owner,
            parent: None,
            depends_on: Vec::new(),
            origin_kind: None,
            source_review: None,
            estimate: None,
            report: None,
            blocked_from: None,
            blocked_reason: None,
            archived_at: None,
            archived_by: None,
            archive_reason: None,
            created_at: at.clone(),
            updated_at: at,
        }
    }

    #[must_use]
    pub fn new_task(
        id: impl Into<String>,
        parent: impl Into<String>,
        title: impl Into<String>,
        intent_owner: impl Into<String>,
        at: impl Into<String>,
    ) -> Self {
        let at = at.into();
        Self {
            id: id.into(),
            kind: Kind::Task,
            state: State::Pending,
            title: title.into(),
            body: "## Task\n".to_owned(),
            created_by: "agent".to_owned(),
            intent_owner: intent_owner.into(),
            parent: Some(parent.into()),
            depends_on: Vec::new(),
            origin_kind: None,
            source_review: None,
            estimate: None,
            report: None,
            blocked_from: None,
            blocked_reason: None,
            archived_at: None,
            archived_by: None,
            archive_reason: None,
            created_at: at.clone(),
            updated_at: at,
        }
    }

    pub fn set_estimate(&mut self, estimate: Estimate) {
        self.estimate = Some(estimate);
    }

    /// Apply a state transition atomically. The card is cloned, mutated and
    /// validated before it is replaced, so a failed guard leaves it unchanged.
    pub fn apply(&mut self, action: Action, reason: Option<&str>) -> Result<()> {
        if action == Action::Unblock {
            let mut next = self.clone();
            let previous = next
                .blocked_from
                .take()
                .ok_or_else(|| invalid(self, "only a blocked batch can be unblocked"))?;
            next.state = previous;
            next.blocked_reason = None;
            next.validate()?;
            *self = next;
            return Ok(());
        }

        if !action.is_state_transition() {
            return Err(invalid(
                self,
                format!("`{action}` is not a card state transition"),
            ));
        }

        let rule = find_transition(self.kind, self.state, action).ok_or_else(|| {
            invalid(
                self,
                format!("illegal transition `{action}` from `{}`", self.state),
            )
        })?;
        let mut next = self.clone();
        let previous = next.state;
        next.state = rule.to;
        if let Some(kind) = rule.to_kind {
            next.kind = kind;
        }

        match action {
            Action::Block => {
                let reason = reason.unwrap_or_default().trim();
                if reason.is_empty() {
                    return Err(invalid(self, "blocking requires a reason"));
                }
                next.blocked_from = Some(previous);
                next.blocked_reason = Some(reason.to_owned());
            }
            Action::Cancel => {
                if reason.unwrap_or_default().trim().is_empty() {
                    return Err(invalid(self, "canceling requires a reason"));
                }
                next.blocked_from = None;
                next.blocked_reason = None;
            }
            _ => {}
        }

        next.validate()?;
        *self = next;
        Ok(())
    }

    #[must_use]
    pub fn legal_actions(&self) -> Vec<Action> {
        let mut actions: Vec<Action> = TRANSITIONS
            .iter()
            .filter(|rule| {
                (rule.from_kind == self.kind
                    || (rule.from_kind == Kind::Proposal && self.kind.is_intent()))
                    && rule.from == self.state
            })
            .map(|rule| rule.action)
            .collect();
        if self.state == State::Blocked {
            actions.push(Action::Unblock);
        }
        actions.sort_unstable();
        actions.dedup();
        actions
    }

    #[must_use]
    pub fn can_apply(&self, action: Action) -> bool {
        let mut probe = self.clone();
        probe.apply(action, Some("probe")).is_ok()
    }

    /// Validate the card's structural invariants.
    pub fn validate(&self) -> Result<()> {
        if !valid_card_id(&self.id, self.kind) {
            return Err(invalid(self, "invalid id for the card kind"));
        }
        if self.title.trim().is_empty() {
            return Err(invalid(self, "title must not be empty"));
        }
        if self.body.trim().is_empty() {
            return Err(invalid(self, "body must not be empty"));
        }
        if self.created_by.trim().is_empty() || self.intent_owner.trim().is_empty() {
            return Err(invalid(
                self,
                "created_by and intent_owner must not be empty",
            ));
        }
        if self.created_at.trim().is_empty() || self.updated_at.trim().is_empty() {
            return Err(invalid(self, "timestamps must not be empty"));
        }
        if !self.state.allowed_for(self.kind) {
            return Err(invalid(
                self,
                format!(
                    "state `{}` is not valid for kind `{}`",
                    self.state, self.kind
                ),
            ));
        }

        match self.kind {
            Kind::Task => {
                let Some(parent) = self.parent.as_deref() else {
                    return Err(invalid(self, "a task requires a parent batch"));
                };
                if !valid_batch_id(parent) {
                    return Err(invalid(self, "task parent is not a batch id"));
                }
                if parent == self.id {
                    return Err(invalid(self, "a task cannot be its own parent"));
                }
                if self
                    .source_review
                    .as_deref()
                    .is_some_and(|id| !id.starts_with("RV-"))
                {
                    return Err(invalid(self, "source_review must be a review artifact id"));
                }
                if self
                    .depends_on
                    .iter()
                    .any(|id| !valid_card_id(id, Kind::Task))
                {
                    return Err(invalid(self, "task dependencies must be task ids"));
                }
                if self.depends_on.iter().any(|id| id == &self.id) {
                    return Err(invalid(self, "a card cannot depend on itself"));
                }
            }
            Kind::Proposal | Kind::Defect | Kind::Research | Kind::Batch => {
                if self.parent.is_some() {
                    return Err(invalid(self, "only tasks have a parent"));
                }
                if self.source_review.is_some() {
                    return Err(invalid(self, "only tasks carry source_review"));
                }
                if self.depends_on.iter().any(|id| id == &self.id) {
                    return Err(invalid(self, "a card cannot depend on itself"));
                }
            }
        }

        if let Some(estimate) = &self.estimate {
            if self.kind.is_intent() {
                return Err(invalid(self, "ideas do not carry estimates"));
            }
            if !estimate.is_valid() {
                return Err(invalid(self, "estimate must be finite and non-negative"));
            }
        }
        if self.kind == Kind::Batch && self.requires_estimate() && self.estimate.is_none() {
            return Err(invalid(self, "a roadmap batch requires an estimate"));
        }

        if self.state == State::Completed {
            if self.report.as_deref().unwrap_or_default().trim().is_empty() {
                return Err(invalid(self, "a completed batch requires a report"));
            }
        } else if self.report.is_some() {
            return Err(invalid(self, "only completed batches carry a report"));
        }

        if self.state == State::Blocked {
            let Some(previous) = self.blocked_from else {
                return Err(invalid(
                    self,
                    "a blocked batch must record its return state",
                ));
            };
            if !matches!(
                previous,
                State::Accepted
                    | State::Roadmap
                    | State::Breakdown
                    | State::Doing
                    | State::Handover
            ) {
                return Err(invalid(self, "blocked return state is not a batch state"));
            }
            if self
                .blocked_reason
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
            {
                return Err(invalid(self, "a blocked batch requires a reason"));
            }
        } else if self.blocked_from.is_some() || self.blocked_reason.is_some() {
            return Err(invalid(self, "only a blocked batch carries blocked fields"));
        }

        Ok(())
    }

    const fn requires_estimate(&self) -> bool {
        matches!(
            self.state,
            State::Roadmap | State::Breakdown | State::Doing | State::Handover | State::Completed
        ) || (matches!(self.state, State::Blocked)
            && matches!(
                self.blocked_from,
                Some(State::Roadmap | State::Breakdown | State::Doing | State::Handover)
            ))
    }
}

/// A human review verdict for one batch round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    Accepted,
    ChangesRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VoteStatus {
    Open,
    Passed,
    Rejected,
    Vetoed,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VoteChoice {
    Approve,
    Reject,
    Abstain,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberVote {
    pub name: String,
    pub email: String,
    pub seats: u32,
    pub choice: VoteChoice,
    #[serde(default)]
    pub recorded_by: Option<String>,
    #[serde(default)]
    pub recorded_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vote {
    pub id: String,
    pub card: String,
    pub opened_by: String,
    pub opened_at: String,
    #[serde(default)]
    pub closed_at: Option<String>,
    pub status: VoteStatus,
    pub quorum_seats: u32,
    pub allocated_seats: u32,
    #[serde(default)]
    pub counted_seats: u32,
    #[serde(default)]
    pub votes: Vec<MemberVote>,
    #[serde(default)]
    pub veto: bool,
    #[serde(default)]
    pub veto_by: Option<String>,
    #[serde(default)]
    pub veto_reason: Option<String>,
    #[serde(default)]
    pub tie_break: Option<VoteChoice>,
    #[serde(default)]
    pub decision: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

impl Vote {
    pub fn validate(&self) -> Result<()> {
        if !self.id.starts_with("V-") {
            return Err(Error::Message("vote id must start with V-".to_owned()));
        }
        if !valid_batch_id(&self.card) {
            return Err(Error::Message("vote card is not a batch id".to_owned()));
        }
        if self.opened_by.trim().is_empty() || self.opened_at.trim().is_empty() {
            return Err(Error::Message(
                "vote opened_by and opened_at are required".to_owned(),
            ));
        }
        if self.allocated_seats < self.counted_seats {
            return Err(Error::Message(
                "vote counted seats exceed allocated seats".to_owned(),
            ));
        }
        if self.status != VoteStatus::Open && self.closed_at.is_none() {
            return Err(Error::Message("a closed vote needs closed_at".to_owned()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OrderStatus {
    Draft,
    Active,
    Retired,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderScope {
    #[serde(default)]
    pub project: bool,
    #[serde(default)]
    pub kinds: Vec<Kind>,
    #[serde(default)]
    pub states: Vec<State>,
    #[serde(default)]
    pub cards: Vec<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub until: Option<String>,
}

impl OrderScope {
    pub fn validate(&self) -> Result<()> {
        if self.project
            && (!self.kinds.is_empty()
                || !self.states.is_empty()
                || !self.cards.is_empty()
                || self.from.is_some()
                || self.until.is_some())
        {
            return Err(Error::Message(
                "project scope cannot be combined with other scopes".to_owned(),
            ));
        }
        if !self.project
            && self.kinds.is_empty()
            && self.states.is_empty()
            && self.cards.is_empty()
            && self.from.is_none()
            && self.until.is_none()
        {
            return Err(Error::Message(
                "an order needs project scope or at least one selector".to_owned(),
            ));
        }
        if let (Some(from), Some(until)) = (&self.from, &self.until)
            && from > until
        {
            return Err(Error::Message(
                "order scope start must not be after its end".to_owned(),
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn applies_to(&self, card: &ProcessCard, today: &str) -> bool {
        if self.project {
            return true;
        }
        if !self.kinds.is_empty() && !self.kinds.contains(&card.kind) {
            return false;
        }
        if !self.states.is_empty() && !self.states.contains(&card.state) {
            return false;
        }
        if !self.cards.is_empty() && !self.cards.contains(&card.id) {
            return false;
        }
        if let Some(from) = &self.from
            && today < from.as_str()
        {
            return false;
        }
        if let Some(until) = &self.until
            && today > until.as_str()
        {
            return false;
        }
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderVersion {
    pub at: String,
    pub by: String,
    pub instruction: String,
    pub status: OrderStatus,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Order {
    pub id: String,
    pub title: String,
    pub status: OrderStatus,
    pub scope: OrderScope,
    #[serde(default)]
    pub instruction: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub consequences: String,
    pub authorised_by: String,
    #[serde(default)]
    pub authorisation: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub history: Vec<OrderVersion>,
}

impl Order {
    pub fn parse(path: &std::path::Path, text: &str) -> Result<Self> {
        let (yaml, body) = split_front_matter(path, text)?;
        let mut order: Self = serde_yaml::from_str(&yaml)
            .map_err(|error| Error::card(path, format!("invalid front matter: {error}")))?;
        order.instruction = body.trim().to_owned();
        order.validate()?;
        Ok(order)
    }

    pub fn to_markdown(&self) -> Result<String> {
        let mut value = serde_yaml::to_value(self)?;
        if let Some(table) = value.as_mapping_mut() {
            table.remove(serde_yaml::Value::String("instruction".to_owned()));
        }
        let yaml = serde_yaml::to_string(&value)?;
        Ok(format!(
            "---
{}---

{}
",
            yaml,
            self.instruction.trim()
        ))
    }

    pub fn validate(&self) -> Result<()> {
        if !self.id.starts_with("SO-") {
            return Err(Error::Message("order id must start with SO-".to_owned()));
        }
        if self.title.trim().is_empty() {
            return Err(Error::Message("order title is required".to_owned()));
        }
        if self.instruction.trim().is_empty() {
            return Err(Error::Message("order instruction is required".to_owned()));
        }
        if self.authorised_by.trim().is_empty() {
            return Err(Error::Message("order authorised_by is required".to_owned()));
        }
        if self.created_at.trim().is_empty() || self.updated_at.trim().is_empty() {
            return Err(Error::Message(
                "order created_at and updated_at are required".to_owned(),
            ));
        }
        self.scope.validate()?;
        Ok(())
    }

    #[must_use]
    pub fn applies_to(&self, card: &ProcessCard, today: &str) -> bool {
        self.status == OrderStatus::Active && self.scope.applies_to(card, today)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewArtifact {
    pub id: String,
    pub batch: String,
    pub round: u32,
    pub reviewer: String,
    pub authored_by: String,
    #[serde(default)]
    pub transcribed_by: Option<String>,
    pub verdict: Verdict,
    #[serde(default)]
    pub findings: Vec<Finding>,
    pub created_at: String,
}

impl ReviewArtifact {
    pub fn validate(&self) -> Result<()> {
        if !self.id.starts_with("RV-") {
            return Err(Error::Message("review id must start with RV-".to_owned()));
        }
        if !valid_batch_id(&self.batch) {
            return Err(Error::Message("review batch is not a batch id".to_owned()));
        }
        if self.round == 0 {
            return Err(Error::Message(
                "review round must be greater than zero".to_owned(),
            ));
        }
        if self.reviewer.trim().is_empty() || self.authored_by.trim().is_empty() {
            return Err(Error::Message(
                "reviewer and authored_by must not be empty".to_owned(),
            ));
        }
        if self.created_at.trim().is_empty() {
            return Err(Error::Message(
                "review timestamp must not be empty".to_owned(),
            ));
        }
        if self.verdict == Verdict::ChangesRequired && self.findings.is_empty() {
            return Err(Error::Message(
                "a changes-required review needs at least one finding".to_owned(),
            ));
        }
        for finding in &self.findings {
            if finding.id.trim().is_empty() || finding.text.trim().is_empty() {
                return Err(Error::Message(
                    "finding id and text must not be empty".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

/// An append-only event record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub id: String,
    pub at: String,
    pub action: String,
    #[serde(default)]
    pub card: Option<String>,
    #[serde(default)]
    pub from: Option<State>,
    #[serde(default)]
    pub to: Option<State>,
    pub actor: String,
    pub executor: String,
    #[serde(default)]
    pub authorised_by: Option<String>,
    #[serde(default)]
    pub authorization: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl Event {
    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty()
            || self.at.trim().is_empty()
            || self.action.trim().is_empty()
            || self.actor.trim().is_empty()
            || self.executor.trim().is_empty()
        {
            return Err(Error::Message(
                "event id, time, action, actor and executor are required".to_owned(),
            ));
        }
        if let Some(card) = &self.card
            && !valid_any_card_id(card)
        {
            return Err(Error::Message("event card id is invalid".to_owned()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MeetingKind {
    Consortium,
    BreakdownClarification,
    HandoverReview,
    ReleaseReadiness,
    Recovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MeetingStatus {
    Pending,
    Accepted,
    Deferred,
    Dismissed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meeting {
    pub id: String,
    pub kind: MeetingKind,
    pub status: MeetingStatus,
    pub trigger: String,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub linked_cards: Vec<String>,
    #[serde(default)]
    pub questions: Vec<Question>,
    #[serde(default)]
    pub decision: Option<String>,
    #[serde(default)]
    pub attendees: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Meeting {
    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() || self.trigger.trim().is_empty() {
            return Err(Error::Message(
                "meeting id and trigger are required".to_owned(),
            ));
        }
        if self.linked_cards.iter().any(|id| !valid_any_card_id(id)) {
            return Err(Error::Message(
                "meeting linked card id is invalid".to_owned(),
            ));
        }
        if self.status == MeetingStatus::Deferred
            && self.due.as_deref().unwrap_or_default().is_empty()
        {
            return Err(Error::Message(
                "a deferred meeting needs a due date".to_owned(),
            ));
        }
        if matches!(
            self.status,
            MeetingStatus::Accepted | MeetingStatus::Dismissed
        ) && self
            .decision
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            return Err(Error::Message(
                "an accepted or dismissed meeting needs a decision".to_owned(),
            ));
        }
        for question in &self.questions {
            if question.id.trim().is_empty() || question.text.trim().is_empty() {
                return Err(Error::Message(
                    "question id and text must not be empty".to_owned(),
                ));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn is_pending(&self) -> bool {
        matches!(
            self.status,
            MeetingStatus::Pending | MeetingStatus::Deferred
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IterationStatus {
    Open,
    Merged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamRecord {
    pub iteration: u64,
    pub status: IterationStatus,
    pub base: String,
    pub writer: String,
    pub lease_expires: String,
    pub started_at: String,
    #[serde(default)]
    pub merged_at: Option<String>,
}

impl UpstreamRecord {
    pub fn validate(&self) -> Result<()> {
        if self.iteration == 0 {
            return Err(Error::Message(
                "upstream iteration must be greater than zero".to_owned(),
            ));
        }
        if self.base.trim().is_empty()
            || self.writer.trim().is_empty()
            || self.started_at.trim().is_empty()
        {
            return Err(Error::Message(
                "upstream base, writer and started_at are required".to_owned(),
            ));
        }
        match self.status {
            IterationStatus::Open => {
                if self.lease_expires.trim().is_empty() {
                    return Err(Error::Message(
                        "an open upstream iteration needs a writer lease".to_owned(),
                    ));
                }
                if self.merged_at.is_some() {
                    return Err(Error::Message(
                        "an open upstream iteration cannot have merged_at".to_owned(),
                    ));
                }
            }
            IterationStatus::Merged => {
                if self
                    .merged_at
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    return Err(Error::Message(
                        "a merged upstream iteration needs merged_at".to_owned(),
                    ));
                }
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.status == IterationStatus::Open
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoadmapOverride {
    pub limit: u32,
    pub reason: String,
    pub approved_by: String,
    pub expires_at: String,
}

impl RoadmapOverride {
    pub fn validate(&self) -> Result<()> {
        if self.limit == 0 {
            return Err(Error::Message(
                "roadmap override limit must be positive".to_owned(),
            ));
        }
        if self.reason.trim().is_empty() || self.approved_by.trim().is_empty() {
            return Err(Error::Message(
                "roadmap override needs a reason and approver".to_owned(),
            ));
        }
        if self.expires_at.trim().is_empty() {
            return Err(Error::Message(
                "roadmap override needs an expiry".to_owned(),
            ));
        }
        Ok(())
    }

    /// Compare RFC 3339 timestamps. Callers should pass UTC (`Z`) timestamps.
    #[must_use]
    pub fn is_active(&self, now: &str) -> bool {
        self.expires_at.as_str() > now
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub id: String,
    pub batch: String,
    pub summary: String,
    #[serde(default)]
    pub tasks: Vec<String>,
    #[serde(default)]
    pub reviews: Vec<String>,
    #[serde(default)]
    pub commits: Vec<String>,
    #[serde(default)]
    pub tests: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub estimated_cost_eur: Option<f64>,
    pub created_at: String,
}

impl Report {
    pub fn validate(&self) -> Result<()> {
        if !self.id.starts_with('R') || !valid_batch_id(&self.batch) {
            return Err(Error::Message(
                "report id or batch id is invalid".to_owned(),
            ));
        }
        if self.summary.trim().is_empty() || self.created_at.trim().is_empty() {
            return Err(Error::Message(
                "report summary and timestamp are required".to_owned(),
            ));
        }
        if self
            .estimated_cost_eur
            .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
        {
            return Err(Error::Message(
                "report cost must be finite and non-negative".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalMode {
    #[default]
    Chat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cadence {
    Weekly,
    Biweekly,
    Monthly,
    OnDemand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeetingCadence {
    #[serde(default = "default_consortium_cadence")]
    pub consortium: Cadence,
    #[serde(default = "default_release_cadence")]
    pub release: Cadence,
}

const fn default_consortium_cadence() -> Cadence {
    Cadence::Weekly
}

const fn default_release_cadence() -> Cadence {
    Cadence::Biweekly
}

impl Default for MeetingCadence {
    fn default() -> Self {
        Self {
            consortium: default_consortium_cadence(),
            release: default_release_cadence(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Branches {
    #[serde(default = "default_remote")]
    pub remote: String,
    #[serde(default = "default_stable")]
    pub stable: String,
    #[serde(default = "default_nightly")]
    pub nightly: String,
    #[serde(default = "default_upstream")]
    pub upstream: String,
}

fn default_remote() -> String {
    "origin".to_owned()
}

fn default_stable() -> String {
    "stable".to_owned()
}

fn default_nightly() -> String {
    "nightly".to_owned()
}

fn default_upstream() -> String {
    "upstream".to_owned()
}

impl Default for Branches {
    fn default() -> Self {
        Self {
            remote: default_remote(),
            stable: default_stable(),
            nightly: default_nightly(),
            upstream: default_upstream(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotationPerson {
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub organisation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RotationStrategy {
    RoundRobin,
    Random,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rotation {
    pub title: String,
    pub description: String,
    pub strategy: RotationStrategy,
    pub current: String,
    #[serde(default)]
    pub last_rotated: Option<String>,
    #[serde(default)]
    pub auto_rotate_days: Option<u32>,
    #[serde(default)]
    pub remove: Vec<String>,
    #[serde(default)]
    pub add: Vec<RotationPerson>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotationsFile {
    pub schema: u32,
    #[serde(default)]
    pub people: BTreeMap<String, RotationPerson>,
    #[serde(default)]
    pub rotations: BTreeMap<String, Rotation>,
}

impl RotationsFile {
    pub fn effective_participants(&self, key: &str) -> Result<Vec<&RotationPerson>> {
        let rotation = self
            .rotations
            .get(key)
            .ok_or_else(|| Error::Message(format!("rotation `{key}` not found")))?;
        let mut people: Vec<&RotationPerson> = self
            .people
            .values()
            .filter(|person| {
                !rotation
                    .remove
                    .iter()
                    .any(|removed| removed.eq_ignore_ascii_case(&person.email))
            })
            .collect();
        people.extend(rotation.add.iter());
        Ok(people)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextConfig {
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "default_handover_reserve")]
    pub handover_reserve_percent: u8,
}

const fn default_handover_reserve() -> u8 {
    20
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            handover_reserve_percent: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualityGateConfig {
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "yes")]
    pub check_source: bool,
    #[serde(default = "yes")]
    pub require_override_reason: bool,
    #[serde(default = "default_report_findings")]
    pub report_threshold_findings: u32,
    #[serde(default = "default_report_chars")]
    pub report_threshold_chars: u32,
}

const fn yes() -> bool {
    true
}
const fn default_report_findings() -> u32 {
    5
}
const fn default_report_chars() -> u32 {
    2000
}

impl Default for QualityGateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            check_source: true,
            require_override_reason: true,
            report_threshold_findings: 5,
            report_threshold_chars: 2000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StakeholderConfig {
    #[serde(default = "default_max_seats")]
    pub max_seats: u32,
    #[serde(default = "default_quorum_seats")]
    pub quorum_seats: u32,
    #[serde(default = "default_seat_term_days")]
    pub seat_term_days: u32,
    #[serde(default = "default_seat_warning_days")]
    pub seat_warning_days: u32,
    #[serde(default)]
    pub members: Vec<Stakeholder>,
}

const fn default_max_seats() -> u32 {
    5
}
const fn default_quorum_seats() -> u32 {
    3
}
const fn default_seat_term_days() -> u32 {
    365
}
const fn default_seat_warning_days() -> u32 {
    90
}

impl Default for StakeholderConfig {
    fn default() -> Self {
        Self {
            max_seats: default_max_seats(),
            quorum_seats: default_quorum_seats(),
            seat_term_days: default_seat_term_days(),
            seat_warning_days: default_seat_warning_days(),
            members: Vec::new(),
        }
    }
}

impl StakeholderConfig {
    #[must_use]
    pub fn total_seats(&self) -> u32 {
        self.members.iter().map(|member| member.seats).sum()
    }

    pub fn validate(&self) -> Result<()> {
        if self.max_seats == 0 {
            return Err(Error::Message(
                "stakeholder max_seats must be greater than zero".to_owned(),
            ));
        }
        if self.quorum_seats > self.max_seats {
            return Err(Error::Message(
                "stakeholder quorum_seats must not exceed max_seats".to_owned(),
            ));
        }
        if self.seat_term_days == 0 || self.seat_warning_days == 0 {
            return Err(Error::Message(
                "stakeholder seat term and warning days must be greater than zero".to_owned(),
            ));
        }
        let total = self.total_seats();
        if total > self.max_seats {
            return Err(Error::Message(format!(
                "stakeholder seats total {total} exceeds max_seats {}",
                self.max_seats
            )));
        }
        let mut emails = BTreeSet::new();
        for member in &self.members {
            if member.name.trim().is_empty() || !member.email.contains('@') {
                return Err(Error::Message(
                    "stakeholder name and email are required".to_owned(),
                ));
            }
            if member.seats == 0 {
                return Err(Error::Message(
                    "stakeholder seats must be at least one".to_owned(),
                ));
            }
            if member.seat_until.trim().is_empty() {
                return Err(Error::Message(
                    "stakeholder seat_until is required".to_owned(),
                ));
            }
            if !emails.insert(member.email.to_lowercase()) {
                return Err(Error::Message(format!(
                    "duplicate stakeholder email `{}`",
                    member.email
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stakeholder {
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub organisation: String,
    pub seats: u32,
    pub seat_until: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guardian {
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub organisation: String,
    #[serde(default)]
    pub ssh_key: Option<String>,
    #[serde(default)]
    pub ssh_fingerprint: Option<String>,
    #[serde(default)]
    pub co_guardians: Vec<CoGuardian>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoGuardian {
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub organisation: String,
    #[serde(default)]
    pub approved_by: Option<String>,
    #[serde(default)]
    pub ssh_key: Option<String>,
    #[serde(default)]
    pub ssh_fingerprint: Option<String>,
}

impl Default for Guardian {
    fn default() -> Self {
        Self {
            name: "Owner".to_owned(),
            email: "owner@example.com".to_owned(),
            organisation: String::new(),
            ssh_key: None,
            ssh_fingerprint: None,
            co_guardians: Vec::new(),
        }
    }
}

impl Guardian {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() || !self.email.contains('@') {
            return Err(Error::Message(
                "guardian name and email are required".to_owned(),
            ));
        }
        if self.ssh_key.is_some() != self.ssh_fingerprint.is_some() {
            return Err(Error::Message(
                "guardian ssh_key and ssh_fingerprint must be set together".to_owned(),
            ));
        }
        let mut emails = BTreeSet::new();
        emails.insert(self.email.to_lowercase());
        for co in &self.co_guardians {
            if co.name.trim().is_empty() || !co.email.contains('@') {
                return Err(Error::Message(
                    "co-guardian name and email are required".to_owned(),
                ));
            }
            if co.ssh_key.is_some() != co.ssh_fingerprint.is_some() {
                return Err(Error::Message(
                    "co-guardian ssh_key and ssh_fingerprint must be set together".to_owned(),
                ));
            }
            if !emails.insert(co.email.to_lowercase()) {
                return Err(Error::Message(format!(
                    "duplicate guardian email `{}`",
                    co.email
                )));
            }
        }
        Ok(())
    }
}

/// The domain policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    #[serde(default)]
    pub branches: Branches,
    #[serde(default = "default_roadmap_limit")]
    pub roadmap_limit: u32,
    #[serde(default)]
    pub approval: ApprovalMode,
    #[serde(default)]
    pub meetings: MeetingCadence,
    #[serde(default)]
    pub costs: CostConfig,
    #[serde(default)]
    pub stakeholders: StakeholderConfig,
    #[serde(default)]
    pub guardian: Guardian,
    #[serde(default)]
    pub quality_gate: QualityGateConfig,
    #[serde(default)]
    pub context: ContextConfig,
}

const fn default_roadmap_limit() -> u32 {
    5
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            branches: Branches::default(),
            roadmap_limit: default_roadmap_limit(),
            approval: ApprovalMode::default(),
            meetings: MeetingCadence::default(),
            costs: CostConfig::default(),
            stakeholders: StakeholderConfig::default(),
            guardian: Guardian::default(),
            quality_gate: QualityGateConfig::default(),
            context: ContextConfig::default(),
        }
    }
}

impl Policy {
    pub fn validate(&self) -> Result<()> {
        if self.branches.stable.trim().is_empty()
            || self.branches.nightly.trim().is_empty()
            || self.branches.upstream.trim().is_empty()
        {
            return Err(Error::Message(
                "stable, nightly and upstream branch names are required".to_owned(),
            ));
        }
        if self.branches.stable == self.branches.nightly
            || self.branches.stable == self.branches.upstream
            || self.branches.nightly == self.branches.upstream
        {
            return Err(Error::Message(
                "stable, nightly and upstream branch names must differ".to_owned(),
            ));
        }
        if self.roadmap_limit == 0 {
            return Err(Error::Message(
                "roadmap limit must be greater than zero".to_owned(),
            ));
        }
        if self.costs.subscription.trim().is_empty() {
            return Err(Error::Message(
                "cost subscription must not be empty".to_owned(),
            ));
        }
        if !self.costs.eur_per_million_tokens.is_finite() || self.costs.eur_per_million_tokens < 0.0
        {
            return Err(Error::Message(
                "cost per million tokens must be finite and non-negative".to_owned(),
            ));
        }
        self.stakeholders.validate()?;
        self.guardian.validate()?;
        Ok(())
    }

    /// The effective roadmap limit. RFC 3339 UTC timestamps are expected.
    #[must_use]
    pub fn effective_roadmap_limit(
        &self,
        override_record: Option<&RoadmapOverride>,
        now: &str,
    ) -> u32 {
        match override_record {
            Some(record) if record.is_active(now) && record.limit > self.roadmap_limit => {
                record.limit
            }
            _ => self.roadmap_limit,
        }
    }
}

fn split_front_matter(path: &std::path::Path, text: &str) -> Result<(String, String)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err(Error::card(path, "missing YAML front matter opener `---`"));
    }

    let mut yaml = String::new();
    let mut body = String::new();
    let mut in_body = false;
    for line in lines {
        if !in_body && line.trim_end() == "---" {
            in_body = true;
            continue;
        }
        if in_body {
            body.push_str(line);
            body.push('\n');
        } else {
            yaml.push_str(line);
            yaml.push('\n');
        }
    }

    if !in_body {
        return Err(Error::card(path, "missing YAML front matter closer `---`"));
    }
    Ok((yaml, body))
}

fn invalid(card: &ProcessCard, message: impl Into<String>) -> Error {
    Error::Message(format!("process card `{}`: {}", card.id, message.into()))
}

fn id_parts(id: &str) -> Option<(&str, Option<&str>)> {
    let rest = id.strip_prefix('#')?;
    let mut parts = rest.split('-');
    let base = parts.next()?;
    let suffix = parts.next();
    if parts.next().is_some() {
        return None;
    }
    if base.len() < 4 || !base.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    if let Some(suffix) = suffix
        && (suffix.len() != 2 || !suffix.chars().all(|ch| ch.is_ascii_digit()))
    {
        return None;
    }
    Some((base, suffix))
}

#[must_use]
pub fn valid_card_id(id: &str, kind: Kind) -> bool {
    match id_parts(id) {
        Some((_, suffix)) => match kind {
            Kind::Task => suffix.is_some(),
            Kind::Proposal | Kind::Defect | Kind::Research | Kind::Batch => suffix.is_none(),
        },
        None => false,
    }
}

#[must_use]
pub fn valid_batch_id(id: &str) -> bool {
    matches!(id_parts(id), Some((_, None)))
}

#[must_use]
pub fn valid_any_card_id(id: &str) -> bool {
    valid_batch_id(id) || valid_card_id(id, Kind::Task)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idea() -> ProcessCard {
        ProcessCard::new_idea("#0007", "CSV export", "owner", "2026-10-24T09:00:00Z")
    }

    fn accepted_batch() -> ProcessCard {
        let mut card = idea();
        card.apply(Action::Refine, None).unwrap();
        card.apply(Action::Pitch, None).unwrap();
        card.apply(Action::AcceptIdea, None).unwrap();
        card
    }

    fn estimated_batch() -> ProcessCard {
        let mut card = accepted_batch();
        card.set_estimate(Estimate::from_parts(
            (1_000.0, 2_000.0, 3_000.0),
            (1.0, 2.0, 3.0),
        ));
        card
    }

    fn handover_batch() -> ProcessCard {
        let mut card = estimated_batch();
        card.apply(Action::Roadmap, None).unwrap();
        card.apply(Action::Breakdown, None).unwrap();
        card.apply(Action::Start, None).unwrap();
        card.apply(Action::Handover, None).unwrap();
        card
    }

    #[test]
    fn state_and_kind_sets_are_consistent() {
        assert_eq!(State::ALL.len(), 17);
        for kind in Kind::ALL {
            for state in kind.states() {
                assert!(state.allowed_for(kind));
            }
        }
        assert!(State::Canceled.allowed_for(Kind::Proposal));
        assert!(State::Canceled.allowed_for(Kind::Batch));
        assert!(State::Canceled.allowed_for(Kind::Task));
        assert!(!State::Blocked.allowed_for(Kind::Task));
        assert!(!State::Error.allowed_for(Kind::Batch));
    }

    #[test]
    fn idea_acceptance_promotes_to_batch() {
        let card = accepted_batch();
        assert_eq!(card.kind, Kind::Batch);
        assert_eq!(card.state, State::Accepted);
        assert!(card.estimate.is_none());
    }

    #[test]
    fn roadmap_requires_an_estimate() {
        let mut card = accepted_batch();
        assert!(card.apply(Action::Roadmap, None).is_err());
        card.set_estimate(Estimate::from_parts(
            (1_000.0, 2_000.0, 3_000.0),
            (1.0, 2.0, 3.0),
        ));
        card.apply(Action::Roadmap, None).unwrap();
        assert_eq!(card.state, State::Roadmap);
    }

    #[test]
    fn block_and_unblock_round_trip() {
        let mut card = estimated_batch();
        card.apply(Action::Block, Some("waiting for credentials"))
            .unwrap();
        assert_eq!(card.state, State::Blocked);
        assert_eq!(card.blocked_from, Some(State::Accepted));
        assert_eq!(
            card.blocked_reason.as_deref(),
            Some("waiting for credentials")
        );
        card.apply(Action::Unblock, None).unwrap();
        assert_eq!(card.state, State::Accepted);
        assert!(card.blocked_from.is_none());
        assert!(card.blocked_reason.is_none());
    }

    #[test]
    fn blocked_batch_can_be_canceled() {
        let mut card = estimated_batch();
        card.apply(Action::Block, Some("external dependency"))
            .unwrap();
        card.apply(Action::Cancel, Some("superseded")).unwrap();
        assert_eq!(card.state, State::Canceled);
    }

    #[test]
    fn completion_requires_a_report() {
        let mut card = handover_batch();
        assert!(card.apply(Action::Complete, None).is_err());
        card.report = Some(".mdboard/reports/R0007.md".to_owned());
        card.apply(Action::Complete, None).unwrap();
        assert_eq!(card.state, State::Completed);
    }

    #[test]
    fn task_lifecycle_and_dependencies() {
        let mut task = ProcessCard::new_task(
            "#0007-01",
            "#0007",
            "Endpoint",
            "owner",
            "2026-10-24T09:30:00Z",
        );
        task.apply(Action::Work, None).unwrap();
        assert_eq!(task.state, State::Working);
        task.apply(Action::Done, None).unwrap();
        assert_eq!(task.state, State::Done);
        assert!(!task.can_apply(Action::Work));

        let mut invalid_task = ProcessCard::new_task(
            "#0007-01",
            "#0007",
            "Endpoint",
            "owner",
            "2026-10-24T09:30:00Z",
        );
        invalid_task.depends_on = vec!["#0007-01".to_owned()];
        assert!(invalid_task.validate().is_err());
    }

    #[test]
    fn task_error_can_be_recovered() {
        let mut task = ProcessCard::new_task(
            "#0007-02",
            "#0007",
            "Tests",
            "owner",
            "2026-10-24T09:45:00Z",
        );
        task.apply(Action::Work, None).unwrap();
        task.apply(Action::Fail, Some("missing fixture")).unwrap();
        assert_eq!(task.state, State::Error);
        task.apply(Action::Recover, Some("fixture provided"))
            .unwrap();
        assert_eq!(task.state, State::Pending);
    }

    #[test]
    fn illegal_transitions_are_rejected() {
        let mut card = idea();
        assert!(card.apply(Action::Complete, None).is_err());
        assert!(card.apply(Action::Work, None).is_err());
        assert!(card.apply(Action::Merge, None).is_err());
    }

    #[test]
    fn transition_table_has_no_duplicate_keys() {
        let mut keys = std::collections::BTreeSet::new();
        for rule in TRANSITIONS {
            assert!(
                keys.insert((rule.from_kind, rule.from, rule.action)),
                "duplicate transition {}/{} -> {}",
                rule.from_kind,
                rule.from,
                rule.action
            );
            assert!(!rule.guard.is_empty());
        }
    }

    #[test]
    fn legal_actions_are_derived_from_the_table() {
        let spark = idea();
        assert!(spark.legal_actions().contains(&Action::Refine));
        assert!(spark.legal_actions().contains(&Action::Cancel));
        assert!(!spark.legal_actions().contains(&Action::Complete));

        let mut blocked = estimated_batch();
        blocked.apply(Action::Block, Some("waiting")).unwrap();
        assert!(blocked.legal_actions().contains(&Action::Unblock));
        assert!(blocked.legal_actions().contains(&Action::Cancel));
    }

    #[test]
    fn ids_follow_the_kind_namespace() {
        assert!(valid_card_id("#0007", Kind::Proposal));
        assert!(valid_card_id("#0007", Kind::Batch));
        assert!(valid_card_id("#0007-01", Kind::Task));
        assert!(!valid_card_id("#0007-01", Kind::Batch));
        assert!(!valid_card_id("#0007", Kind::Task));
        assert!(!valid_card_id("0007", Kind::Proposal));
        assert!(!valid_card_id("#7", Kind::Proposal));
        assert!(valid_batch_id("#0007"));
        assert!(!valid_batch_id("#0007-01"));
    }

    #[test]
    fn policy_defaults_and_override() {
        let policy = Policy::default();
        assert_eq!(policy.roadmap_limit, 5);
        assert_eq!(policy.approval, ApprovalMode::Chat);
        assert_eq!(policy.meetings.consortium, Cadence::Weekly);
        assert_eq!(policy.meetings.release, Cadence::Biweekly);
        policy.validate().unwrap();

        let record = RoadmapOverride {
            limit: 8,
            reason: "release crunch".to_owned(),
            approved_by: "owner".to_owned(),
            expires_at: "2026-11-01T00:00:00Z".to_owned(),
        };
        assert_eq!(
            policy.effective_roadmap_limit(Some(&record), "2026-10-24T12:00:00Z"),
            8
        );
        assert_eq!(
            policy.effective_roadmap_limit(Some(&record), "2026-11-02T12:00:00Z"),
            5
        );
    }

    #[test]
    fn review_artifact_validation() {
        let artifact = ReviewArtifact {
            id: "RV-0007-01-01".to_owned(),
            batch: "#0007".to_owned(),
            round: 1,
            reviewer: "reviewer".to_owned(),
            authored_by: "reviewer".to_owned(),
            transcribed_by: Some("agent".to_owned()),
            verdict: Verdict::ChangesRequired,
            findings: vec![Finding {
                id: "F1".to_owned(),
                text: "Streaming response closes before the final row.".to_owned(),
            }],
            created_at: "2026-10-24T15:00:00Z".to_owned(),
        };
        artifact.validate().unwrap();

        let mut invalid = artifact;
        invalid.findings.clear();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn upstream_record_validation() {
        let mut record = UpstreamRecord {
            iteration: 1,
            status: IterationStatus::Open,
            base: "abc1234".to_owned(),
            writer: "agent".to_owned(),
            lease_expires: "2026-10-24T18:00:00Z".to_owned(),
            started_at: "2026-10-24T09:00:00Z".to_owned(),
            merged_at: None,
        };
        record.validate().unwrap();
        record.status = IterationStatus::Merged;
        assert!(record.validate().is_err());
        record.merged_at = Some("2026-10-24T17:00:00Z".to_owned());
        record.validate().unwrap();
    }

    #[test]
    fn events_round_trip() {
        let event = Event {
            id: "E-0007-work".to_owned(),
            at: "2026-10-24T10:00:00Z".to_owned(),
            action: "work".to_owned(),
            card: Some("#0007-01".to_owned()),
            from: Some(State::Pending),
            to: Some(State::Working),
            actor: "agent".to_owned(),
            executor: "agent".to_owned(),
            authorised_by: None,
            authorization: None,
            reason: None,
            commit: None,
            metadata: BTreeMap::new(),
        };
        event.validate().unwrap();
        let text = serde_yaml::to_string(&event).unwrap();
        let parsed: Event = serde_yaml::from_str(&text).unwrap();
        assert_eq!(parsed.id, event.id);
        assert_eq!(parsed.to, Some(State::Working));
    }
}

#[cfg(test)]
mod governance_tests {
    use super::*;

    fn member(email: &str, seats: u32) -> Stakeholder {
        Stakeholder {
            name: email.to_owned(),
            email: email.to_owned(),
            organisation: String::new(),
            seats,
            seat_until: "2027-12-31".to_owned(),
        }
    }

    #[test]
    fn stakeholder_seats_sum_and_emails_are_validated() {
        let mut config = StakeholderConfig {
            members: vec![member("a@example.com", 3), member("b@example.com", 3)],
            ..Default::default()
        };
        assert!(config.validate().is_err());
        config.max_seats = 6;
        assert!(config.validate().is_ok());
        config.members.push(member("A@example.com", 1));
        assert!(config.validate().is_err());
    }

    #[test]
    fn guardian_and_co_guardian_emails_are_distinct() {
        let mut guardian = Guardian::default();
        guardian.co_guardians.push(CoGuardian {
            name: "Owner".to_owned(),
            email: "OWNER@example.com".to_owned(),
            organisation: String::new(),
            approved_by: None,
            ssh_key: None,
            ssh_fingerprint: None,
        });
        assert!(guardian.validate().is_err());
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;

    fn card() -> ProcessCard {
        let mut card = ProcessCard::new_idea("#0001", "Example", "owner", "2026-10-28T00:00:00Z");
        card.state = State::Pitch;
        card
    }

    #[test]
    fn project_and_selector_scopes_apply_deterministically() {
        let card = card();
        let project = OrderScope {
            project: true,
            ..OrderScope::default()
        };
        assert!(project.applies_to(&card, "2026-10-28"));
        let wrong = OrderScope {
            kinds: vec![Kind::Batch],
            ..OrderScope::default()
        };
        assert!(!wrong.applies_to(&card, "2026-10-28"));
        let right = OrderScope {
            kinds: vec![Kind::Proposal],
            states: vec![State::Pitch],
            ..OrderScope::default()
        };
        assert!(right.applies_to(&card, "2026-10-28"));
    }

    #[test]
    fn order_markdown_round_trip_keeps_instruction_and_history() {
        let order = Order {
            id: "SO-0001".to_owned(),
            title: "Bump version".to_owned(),
            status: OrderStatus::Active,
            scope: OrderScope {
                project: true,
                ..OrderScope::default()
            },
            instruction: "Bump the minor version.".to_owned(),
            rationale: "Release hygiene.".to_owned(),
            consequences: String::new(),
            authorised_by: "owner".to_owned(),
            authorisation: Some("approved".to_owned()),
            created_at: "2026-10-28T00:00:00Z".to_owned(),
            updated_at: "2026-10-28T00:00:00Z".to_owned(),
            history: Vec::new(),
        };
        let text = order.to_markdown().unwrap();
        let parsed = Order::parse(std::path::Path::new("SO-0001.md"), &text).unwrap();
        assert_eq!(parsed.instruction, order.instruction);
        assert_eq!(parsed.status, OrderStatus::Active);
    }
}

#[cfg(test)]
mod obsolete_tests {
    use super::*;

    #[test]
    fn obsolete_is_task_only_terminal_and_reopenable() {
        assert!(State::Obsolete.allowed_for(Kind::Task));
        assert!(!State::Obsolete.allowed_for(Kind::Proposal));
        assert!(State::Obsolete.is_terminal());
        assert!(
            TRANSITIONS
                .iter()
                .any(|rule| rule.action == Action::Obsolete)
        );
        assert!(
            TRANSITIONS
                .iter()
                .any(|rule| rule.action == Action::ReopenTask)
        );
    }
}

#[cfg(test)]
mod rotation_tests {
    use super::*;

    #[test]
    fn effective_participants_apply_remove_and_add() {
        let mut people = BTreeMap::new();
        people.insert(
            "alice".to_owned(),
            RotationPerson {
                name: "Alice".to_owned(),
                email: "alice@example.com".to_owned(),
                organisation: String::new(),
            },
        );
        people.insert(
            "bob".to_owned(),
            RotationPerson {
                name: "Bob".to_owned(),
                email: "bob@example.com".to_owned(),
                organisation: String::new(),
            },
        );
        let mut rotations = BTreeMap::new();
        rotations.insert(
            "meeting".to_owned(),
            Rotation {
                title: "Meeting".to_owned(),
                description: "Moderate".to_owned(),
                strategy: RotationStrategy::RoundRobin,
                current: "alice@example.com".to_owned(),
                last_rotated: None,
                auto_rotate_days: Some(7),
                remove: vec!["bob@example.com".to_owned()],
                add: vec![RotationPerson {
                    name: "Carol".to_owned(),
                    email: "carol@example.com".to_owned(),
                    organisation: String::new(),
                }],
            },
        );
        let file = RotationsFile {
            schema: 1,
            people,
            rotations,
        };
        let emails: Vec<_> = file
            .effective_participants("meeting")
            .unwrap()
            .iter()
            .map(|person| person.email.as_str())
            .collect();
        assert_eq!(emails, vec!["alice@example.com", "carol@example.com"]);
    }
}

#[cfg(test)]
mod markdown_tests {
    use super::*;

    #[test]
    fn card_markdown_round_trip_preserves_body() {
        let mut card = ProcessCard::new_idea("#0001", "Title", "owner", "2026-10-24T10:00:00Z");
        card.body = "## Intent\n\nKeep me.\n".to_owned();
        let text = card.to_markdown().unwrap();
        assert!(text.starts_with("---\n"));
        assert!(!text.contains("body:"));
        let path = std::path::Path::new("0001.md");
        let parsed = ProcessCard::parse(path, &text).unwrap();
        assert_eq!(parsed.body.trim(), "## Intent\n\nKeep me.");
        assert_eq!(parsed.id, "#0001");
    }
}
