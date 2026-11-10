//! Named actions for the 1.0 process model.
//!
//! The CLI and HTML board call these functions. Every state change is a named
//! action with a guard and an append-only event.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use mdboard_core::process::{
    Action, Authority, Event, Finding, Kind, Meeting, MeetingKind, MeetingStatus, MemberVote,
    Order, OrderScope, OrderStatus, OrderVersion, Policy, ProcessCard, Question, ReviewArtifact,
    StakeholderConfig, State, TRANSITIONS, Verdict, Vote, VoteChoice, VoteStatus,
};
use mdboard_core::{ActorKind, Actors};
use mdboard_driver::process::{ProcessLoaded, now};
use thiserror::Error;

/// The normative agent text. `mdboard runbook` prints this verbatim.
pub const RUNBOOK: &str = r#"# mdBoard agent runbook (1.0 alpha)

You are the agent. Humans own intent and decisions; you operate mdBoard and
implement the work. The humans talk to you; mdBoard coordinates both of you.

## Truth

- The open `upstream` iteration is the working truth.
- `nightly` is the integration train; `stable` is released truth.
- Open a iteration with `mdboard upstream start`; the owner merges with
  `mdboard upstream merge` and releases with `mdboard release`. The agent
  never merges.
- Cards, events, reviews, meetings and reports are facts in Git.
- `BOARD.md`, `ROADMAP.md` and the HTML board are generated views.

## Before acting

Run `mdboard context` and, for a card, `mdboard actions <id>`. Use only named
actions. Never hand-edit a card, event, review, report or generated view.

## Attachments

Card-specific human input lives under `.mdboard/attachments/<id>/`. Keep
`.mdboard/cards/` flat; do not create card directories there. Reference files with
repository-relative paths and inspect them with `mdboard attachment list <id>` and
`mdboard attachment path <id> <name>`. Generated reviews and reports stay in
`.mdboard/reviews/` and `.mdboard/reports/`; `mdboard check` rejects missing,
escaping or orphaned attachment paths.

## Board layout

The `.mdboard/` top level is fixed:

```
.mdboard/
  cards/
  events/
  meetings/
  reviews/
  reports/
  attachments/
  votes/
  orders/
  site/                 # generated
  actors.toml
  upstream.md
  roadmap-override.md
  mirror.toml
  rotations.toml
```

Never create, remove, rename or move a first-level file or directory. Card-specific
notes, research, specifications, mockups and generated evidence belong in
`.mdboard/attachments/<card-id>/`. If no card exists for the material, create a
spark first. Never use `docs/` or a new `.mdboard/<topic>/` directory for card
material. A structural change requires the guardian and a migration, not an
agent choice. `mdboard check` rejects unknown top-level entries.

## Quality gate

Before `draft -> pitch` or `refine -> pitch`, when `[quality_gate] enabled = true`,
run `mdboard quality-gate <card>` with the findings. The gate is a soft gate:
a failed result still allows an explicit forced pitch with a reason.

## Commit identity

There are three identity classes:

- **human** — decision-bearing content: card content, refinement, pitch, acceptance, roadmap, review statements, completion, merge and release.
- **agent** — implementation work: source code, tests and technical implementation documentation.
- **mdboard** — generated output and maintenance: `BOARD.md`, `ROADMAP.md`, `.mdboard/site/`, build metadata and release metadata.

`mdboard commit --identity human|agent|mdboard` selects the class explicitly. When `--identity` is omitted, the executing actor determines the legacy default.

Human identity:

```text
mdboard --actor agent commit --identity human --as owner \
  --authorised-by owner --authorization "Accept the review" -m "..."
```

The human needs `git_name` and `git_email`; an agent executing the commit becomes the committer. Human authorship performed by an agent adds `Authorised-by`, `Authorization` and `Performed-by`.

Agent identity must be executed by an agent and cannot commit generated output.

mdboard identity is built in as `mdboard <no-reply@mdboard.io>`, cannot be used for source code, and adds `Generated-by: mdboard`.

When `--identity` is explicit, mdBoard validates the staged path classes and refuses obvious mismatches. Keep one identity class per commit; do not mix a human decision, agent code and generated output. Existing history is not rewritten. Identities are passed only to that commit through `GIT_AUTHOR_*` / `GIT_COMMITTER_*`; Git config is never mutated.

## People management

Use CLI commands, not hand-edited config:

```text
mdboard person add <id> --display-name ... --role member --email ... --until ...
mdboard actor-remove <id> --retire|--delete --authorised-by owner --authorization "..."
mdboard stakeholder add --name ... --email ... --seats 1 --until ...
mdboard stakeholder remove --email ...
```

- `person add` writes the actor plus optional stakeholder seat and guardian/co-guardian.
- `--retire` keeps history; `--delete` requires no historical references.
- Stakeholder removal is refused while a vote is open.
- Owner authorisation is required for agent execution.
- Writes preserve TOML comments.

## Context exhaustion

Before touching a card for work, and before any long operation, check the remaining
model context against the work at hand. The harness supplies the measurement.

- Default reserve: 20%.
- If the work will not fit, stop before starting it.
- Finish only a safe in-progress unit.
- Print a complete handover prompt with `mdboard handover-prompt [card]`.
- Never claim work is complete when the context ran out.
- The owner may explicitly override; record the scope and warning.
- A new session must recover from the repository, never from memory.

## Publication and privacy (target process)

Cards have one visibility level: `public` or `private`. New cards are public by default.

- Private cards stay in the internal board and reports, but must not appear in `CHANGELOG.md`, the public `changelog/` directory or the public mirror.
- Public cards keep card ids in the changelog. Private-derived public entries use opaque ids with no card id.
- Public issues/PRs may reference public cards only; private work is summarized without ids or links.
- A private batch makes its tasks private. Archived private cards stay private.
- Before any mirror push, run the publication and secret checks; `mirror` must invoke the same checks.
- Never paste private card content, attachments or reports into public text.
- When the agent creates a card, it must mention the default public visibility and how to make the card private.
- Until `#0057`–`#0059` are implemented, do not claim these controls exist; ask the owner before publishing anything that may be private.

## Migrations

Before a migration runs, print one net summary between installed and target versions.
Show new, changed and removed user-facing behaviour, build dates, impact, human action
and risk. Ask the guardian to approve; `mdboard migrate --yes` is the approval.
Never list intermediate steps. Omit features introduced and removed in the same jump.
If the pre-state is unknown or hand-edited, fail closed and show repair options.

## Costs

When the human asks you to adapt the cost model, look up the current pricing
with your own tools. Cite the source and date, name the model and subscription,
and state the input/output mix, peak/off-peak assumption and currency
conversion. Propose the blended EUR rate, then update `[costs]` only with owner
approval. mdBoard does not fetch prices or reconcile invoices.

## Authority

Act alone only for: creating an intent card on explicit human instruction, refining
content while an idea is in `refine`, estimating, breaking down, creating tasks,
working and finishing tasks, and moving a completed batch to `handover`.

For every human-gated action, restate the effect, get an explicit decision, and
record the human authoriser and statement. Gates: accept/reject an idea,
re-estimate, roadmap, start, accept a batch review, block/unblock/cancel, and
branch merge/release.

Never self-approve, never accept your own work, never merge, never release.
The owner executes branch operations. Only one agent writer holds the
`upstream` lease.

## Batch execution

Once a batch is started, execute the whole batch without per-task human prompts:

- use `mdboard next <batch>` to get the dependency-ready task and
  `mdboard batch-status <batch>` to see remaining work;
- work the next pending task in dependency order;
- implement it, run the relevant checks, commit at the task boundary and mark it done;
- immediately start the next pending task;
- stop only for a blocked task, missing information, a failed gate, or a human
  review/merge/release gate;
- when every task is done or obsolete, with at least one done, move the batch to
  `handover` and ask for review.

Do not stop merely because one task finished. Report at meaningful boundaries, not
after every task.

## SSH authentication

Guardian and co-guardian SSH keys are optional.

- No configured key means the roadmap gate is open and the project is in the
  low-security mode; warn the guardian once per pitch session.
- At least one configured key means `accepted -> roadmap` requires a fresh
  challenge signature from the guardian or a co-guardian.
- Never compare public-key text as authentication; only accept a verified
  `ssh-keygen -Y verify` result over a fresh nonce.
- `mdboard guardian ssh-challenge <card>` creates the challenge.
- Sign the nonce with `ssh-keygen -Y sign -f <private-key> -n mdboard <nonce-file>`.
- `mdboard guardian ssh-verify <card> --nonce ... --signature ... --email ...`
  records the verification.
- A malformed configured key fails closed.
- Never store a private key.

## Governance

The guardian is the final human authority. Co-guardians share the guardian role.
Stakeholders hold seats; seats may carry multiple votes, expire and be renewed.

- Without stakeholders, proposals are accepted without a vote.
- With stakeholders, `accept-idea` requires a passed vote.
- Open a vote with `mdboard vote open`.
- Record each result with `mdboard vote record`.
- Resolve with `mdboard vote resolve`; a tie needs a guardian tie-break.
- A guardian or co-guardian may veto with a recorded reason.
- Always ask expired seats; never count them as ordinary abstentions.
- If all seats are expired, the vote auto-passes.
- The guardian and co-guardians share veto, tie-break and transfer authority.
- Use `mdboard stakeholder` and `mdboard guardian` for seat renewal and guardian
  changes; do not hand-edit `mdboard.toml`.

## States

Intent: `draft -> refine -> pitch -> accepted | rejected` (or `draft -> pitch`).
Batch: `accepted -> roadmap -> breakdown -> doing -> handover -> completed`.
Task: `pending -> working -> done`; `working -> error -> pending | canceled`; `pending | working | error -> obsolete`; `obsolete -> pending` on reopen.
`blocked` is a batch exception; `canceled` is shared.

One batch may be in `breakdown` or `doing`; `handover` may hold several batches
for parallel human review. One task may be `working`.

## Review

Review is per batch at `handover`, not per task. A reviewer must differ from the
intent owner. Record `authored_by` and `transcribed_by`; never impersonate a
reviewer. Changes return the batch to `doing` and create rework tasks.

## Meetings

Consortium review is the fixed pitch/roadmap gate. Breakdown clarification,
handover review, recovery and conflict are on-demand. Release readiness is the
fixed release gate. Surface pending meetings in every brief; never silently drop
one.

## Commit

Commit at task and batch boundaries with `mdboard commit -m "…"`. The owner's
merge and release commands refuse a dirty worktree. Do not commit on the
owner's behalf beyond the open upstream iteration.

## Brief

After every transition, run `mdboard brief` and tell the human: what changed,
what needs them, and what happens next.
"#;

pub type Result<T> = std::result::Result<T, ActionError>;

#[derive(Debug, Error)]
pub enum ActionError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Internal(String),
}

impl From<mdboard_core::Error> for ActionError {
    fn from(error: mdboard_core::Error) -> Self {
        Self::BadRequest(error.to_string())
    }
}

impl From<anyhow::Error> for ActionError {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<std::io::Error> for ActionError {
    fn from(error: std::io::Error) -> Self {
        Self::Internal(error.to_string())
    }
}

/// A loaded board plus the authenticated actor.
pub struct Session {
    pub loaded: ProcessLoaded,
    pub actor: String,
    pub roles: Vec<String>,
}

impl Session {
    pub fn load(root: &Path, actor: &str) -> Result<Self> {
        let loaded = ProcessLoaded::load(root)?;
        let actors = Actors::load_optional(&loaded.process_root().join("actors.toml"))?;
        let entry = actors
            .get(actor)
            .ok_or_else(|| ActionError::NotFound(format!("actor `{actor}` is not configured")))?;
        if entry.retired {
            return Err(ActionError::Forbidden(format!(
                "actor `{actor}` is retired"
            )));
        }
        let iteration = loaded
            .upstream
            .as_ref()
            .filter(|iteration| iteration.status == mdboard_core::process::IterationStatus::Open)
            .ok_or_else(|| {
                ActionError::Conflict(
                    "no open upstream iteration; run `mdboard upstream start` first".to_owned(),
                )
            })?;
        if entry.has_role("agent") && !entry.has_role("owner") {
            if iteration.writer != actor {
                return Err(ActionError::Forbidden(format!(
                    "actor `{actor}` does not hold the upstream writer lease"
                )));
            }
            let expired = chrono::DateTime::parse_from_rfc3339(&iteration.lease_expires)
                .map_or(true, |expires| expires <= chrono::Utc::now());
            if expired {
                return Err(ActionError::Forbidden(
                    "the upstream writer lease has expired; renew it first".to_owned(),
                ));
            }
        }
        Ok(Self {
            loaded,
            actor: actor.to_owned(),
            roles: entry.roles.clone(),
        })
    }

    #[must_use]
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|candidate| candidate == role)
    }

    #[must_use]
    pub fn is_agent(&self) -> bool {
        self.has_role("agent")
    }

    #[must_use]
    pub fn is_owner(&self) -> bool {
        self.has_role("owner")
    }

    #[must_use]
    pub fn is_human(&self) -> bool {
        self.has_role("owner") || self.has_role("member")
    }

    pub fn require(&self, id: &str) -> Result<&ProcessCard> {
        self.loaded
            .find(id)
            .ok_or_else(|| ActionError::NotFound(format!("card `{id}` not found")))
    }

    pub fn child_tasks(&self, parent: &str) -> Vec<&ProcessCard> {
        self.loaded
            .cards
            .iter()
            .filter(|card| card.parent.as_deref() == Some(parent))
            .collect()
    }

    pub fn active_batch_excluding(&self, id: &str) -> Option<&ProcessCard> {
        self.loaded.cards.iter().find(|card| {
            card.id != id
                && card.kind == Kind::Batch
                && matches!(card.state, State::Breakdown | State::Doing)
        })
    }

    pub fn working_task_excluding(&self, id: &str) -> Option<&ProcessCard> {
        self.loaded
            .cards
            .iter()
            .find(|card| card.id != id && card.kind == Kind::Task && card.state == State::Working)
    }
}

/// Create an idea spark. The agent may do this only on a human instruction;
/// the instruction itself is not a state transition.
pub fn spark(
    root: &Path,
    actor: &str,
    title: &str,
    body: &str,
    intent_owner: &str,
) -> Result<ProcessCard> {
    new_intent(root, actor, Kind::Proposal, title, body, intent_owner)
}

pub fn new_intent(
    root: &Path,
    actor: &str,
    kind: Kind,
    title: &str,
    body: &str,
    intent_owner: &str,
) -> Result<ProcessCard> {
    let mut session = Session::load(root, actor)?;
    if !session.is_agent() && !session.is_human() {
        return Err(ActionError::Forbidden(
            "only the agent or a human can create a spark".to_owned(),
        ));
    }
    if !kind.is_intent() {
        return Err(ActionError::BadRequest(
            "only proposal, defect and research are intent kinds".to_owned(),
        ));
    }
    let id = session.loaded.next_idea_or_batch_id();
    let mut card = ProcessCard::new_intent(kind, id, title, intent_owner, now());
    card.created_by = actor.to_owned();
    if !body.trim().is_empty() {
        card.body = body.trim().to_owned();
    }
    card.validate()?;
    session.loaded.create_card(card.clone())?;
    let event = Event {
        id: session.loaded.next_event_id("new", Some(&card.id)),
        at: now(),
        action: "new".to_owned(),
        card: Some(card.id.clone()),
        from: None,
        to: Some(card.state),
        actor: actor.to_owned(),
        executor: actor.to_owned(),
        authorised_by: None,
        authorization: None,
        reason: None,
        commit: None,
        metadata: Default::default(),
    };
    session.loaded.write_event(&event)?;
    Ok(card)
}

/// Apply a named state transition with authority and guards.
#[allow(clippy::too_many_arguments)]
pub fn transition(
    root: &Path,
    actor: &str,
    id: &str,
    action: Action,
    reason: Option<&str>,
    authorised_by: Option<&str>,
    authorization: Option<&str>,
    commit: Option<&str>,
) -> Result<ProcessCard> {
    let mut session = Session::load(root, actor)?;
    let current = session.require(id)?.clone();
    let authority = if action == Action::Unblock {
        if current.state != State::Blocked {
            return Err(ActionError::Conflict(
                "only a blocked batch can be unblocked".to_owned(),
            ));
        }
        Authority::Human
    } else {
        TRANSITIONS
            .iter()
            .find(|rule| {
                rule.action == action
                    && rule.from == current.state
                    && (rule.from_kind == current.kind
                        || (rule.from_kind == Kind::Proposal && current.kind.is_intent()))
            })
            .map(|rule| rule.authority)
            .ok_or_else(|| {
                ActionError::Conflict(format!(
                    "`{action}` is not legal from `{}` for a {}",
                    current.state, current.kind
                ))
            })?
    };

    check_authority(&session, authority, authorised_by, authorization)?;
    guard(&session, &current, action, reason, commit)?;

    let mut next = current.clone();
    if action == Action::Complete {
        let path = format!(
            "{}/{}/R{}.md",
            session.loaded.paths.root,
            session.loaded.paths.reports,
            current.id.trim_start_matches('#')
        );
        next.report = Some(path);
    }
    next.apply(action, reason)?;
    if action == Action::AcceptIdea && current.kind.is_intent() {
        next.origin_kind = Some(current.kind);
    }
    next.validate()?;

    if action == Action::Cancel {
        cascade_cancel(&mut session, &next)?;
    }
    session.loaded.save_card(&mut next)?;

    let event = Event {
        id: session
            .loaded
            .next_event_id(action.as_str(), Some(&next.id)),
        at: now(),
        action: action.as_str().to_owned(),
        card: Some(next.id.clone()),
        from: Some(current.state),
        to: Some(next.state),
        actor: actor.to_owned(),
        executor: actor.to_owned(),
        authorised_by: authorised_by.map(str::to_owned),
        authorization: authorization.map(str::to_owned),
        reason: reason.map(str::to_owned),
        commit: commit.map(str::to_owned),
        metadata: Default::default(),
    };
    session.loaded.write_event(&event)?;

    if action == Action::Handover {
        open_handover_meeting(&mut session, &next)?;
    }

    if action == Action::Complete {
        let report_path = next.report.clone().expect("completion sets a report");
        let report = render_report(&session, &next);
        let written = session.loaded.write_report(&next, &report)?;
        session.loaded.append_changelog(&next, &written)?;
        close_handover_meeting(&mut session, &next, "batch completed")?;
        let _ = report_path;
    }

    if action == Action::RequestChanges && next.kind == Kind::Batch {
        create_rework_tasks(&mut session, &next)?;
        close_handover_meeting(&mut session, &next, "changes requested")?;
    }

    Ok(next)
}

fn check_authority(
    session: &Session,
    authority: Authority,
    authorised_by: Option<&str>,
    authorization: Option<&str>,
) -> Result<()> {
    if session.is_agent() {
        if !authority.agent_may_execute() {
            return Err(ActionError::Forbidden(
                "this action must be executed by the human owner, not the agent".to_owned(),
            ));
        }
        if authority == Authority::Agent {
            return Ok(());
        }
        let authorised_by = authorised_by.ok_or_else(|| {
            ActionError::Forbidden("a human authoriser is required for this action".to_owned())
        })?;
        let statement = authorization.unwrap_or_default().trim();
        if statement.is_empty() {
            return Err(ActionError::Forbidden(
                "the human's authorisation statement is required".to_owned(),
            ));
        }
        let actors = Actors::load_optional(&session.loaded.process_root().join("actors.toml"))?;
        let human = actors.get(authorised_by).ok_or_else(|| {
            ActionError::Forbidden(format!("authoriser `{authorised_by}` is not configured"))
        })?;
        if human.kind != ActorKind::Human {
            return Err(ActionError::Forbidden(
                "the authoriser must be a human actor".to_owned(),
            ));
        }
        if human.retired {
            return Err(ActionError::Forbidden(format!(
                "authoriser `{authorised_by}` is retired"
            )));
        }
        if !authority.allows(role_of(human.roles.as_slice())) {
            return Err(ActionError::Forbidden(format!(
                "`{authorised_by}` is not allowed to authorise this action"
            )));
        }
        return Ok(());
    }

    if authority == Authority::Agent {
        return Err(ActionError::Forbidden(
            "this action belongs to the agent".to_owned(),
        ));
    }
    if !authority.allows(role_of(&session.roles)) {
        return Err(ActionError::Forbidden(
            "the actor does not have the required role for this action".to_owned(),
        ));
    }
    Ok(())
}

fn role_of(roles: &[String]) -> mdboard_core::process::Role {
    if roles.iter().any(|role| role == "owner") {
        mdboard_core::process::Role::Owner
    } else if roles.iter().any(|role| role == "agent") {
        mdboard_core::process::Role::Agent
    } else {
        mdboard_core::process::Role::Member
    }
}

fn guard(
    session: &Session,
    card: &ProcessCard,
    action: Action,
    reason: Option<&str>,
    commit: Option<&str>,
) -> Result<()> {
    match action {
        Action::Pitch => {
            if session.loaded.policy.quality_gate.enabled {
                let passed = session.loaded.events.iter().any(|event| {
                    event.card.as_deref() == Some(card.id.as_str())
                        && (event.action == "quality-gate-passed"
                            || event.action == "quality-gate-overridden")
                });
                if !passed {
                    return Err(ActionError::Conflict(
                        "quality gate did not pass; run `mdboard quality-gate` or force the pitch"
                            .to_owned(),
                    ));
                }
            }
        }
        Action::AcceptIdea => {
            if !session.loaded.policy.stakeholders.members.is_empty() {
                let passed = session
                    .loaded
                    .latest_vote_for(&card.id)
                    .is_some_and(|vote| vote.status == VoteStatus::Passed);
                if !passed {
                    return Err(ActionError::Conflict(
                        "a passed stakeholder vote is required before acceptance".to_owned(),
                    ));
                }
            }
        }
        Action::Roadmap => {
            if card.estimate.is_none() {
                return Err(ActionError::Conflict(
                    "a batch needs an estimate before it can enter the roadmap".to_owned(),
                ));
            }
            if session.loaded.roadmap_count() >= session.loaded.roadmap_limit() {
                return Err(ActionError::Conflict(format!(
                    "the roadmap is full ({} of {})",
                    session.loaded.roadmap_count(),
                    session.loaded.roadmap_limit()
                )));
            }
            if has_ssh_keys(&session.loaded)
                && !session.loaded.events.iter().any(|event| {
                    event.action == "ssh-verified"
                        && event.card.as_deref() == Some(card.id.as_str())
                })
            {
                return Err(ActionError::Conflict(
                    "SSH authentication is configured; run `mdboard guardian ssh-challenge` and `ssh-verify` before roadmap"
                        .to_owned(),
                ));
            }
        }
        Action::Unroadmap => {
            if session
                .child_tasks(&card.id)
                .iter()
                .any(|task| task.state != State::Canceled)
            {
                return Err(ActionError::Conflict(
                    "remove tasks from the batch before unroading it".to_owned(),
                ));
            }
        }
        Action::Breakdown => {
            if card.estimate.is_none() {
                return Err(ActionError::Conflict(
                    "a batch needs an estimate before breakdown".to_owned(),
                ));
            }
            if let Some(other) = session.active_batch_excluding(&card.id) {
                return Err(ActionError::Conflict(format!(
                    "another batch is active: {} ({})",
                    other.id, other.state
                )));
            }
        }
        Action::Start => {
            if session.child_tasks(&card.id).is_empty() {
                return Err(ActionError::Conflict(
                    "create at least one task before starting".to_owned(),
                ));
            }
            if session.loaded.open_clarification_count(&card.id) > 0 {
                return Err(ActionError::Conflict(
                    "the breakdown clarification is still open".to_owned(),
                ));
            }
        }
        Action::Handover => {
            let tasks = session.child_tasks(&card.id);
            if tasks.is_empty()
                || tasks
                    .iter()
                    .any(|task| !matches!(task.state, State::Done | State::Obsolete))
                || !tasks.iter().any(|task| task.state == State::Done)
            {
                return Err(ActionError::Conflict(
                    "all tasks must be done or obsolete, with at least one done, before handover"
                        .to_owned(),
                ));
            }
        }
        Action::Obsolete => {
            if reason.unwrap_or_default().trim().is_empty() {
                return Err(ActionError::Conflict(
                    "a reason is required for obsolete".to_owned(),
                ));
            }
            if let Some(parent) = card.parent.as_deref() {
                let parent = session.require(parent)?;
                if parent.state != State::Doing {
                    return Err(ActionError::Conflict(
                        "the parent batch is not doing; use unblock or request-changes first"
                            .to_owned(),
                    ));
                }
            }
        }
        Action::ReopenTask => {
            if reason.unwrap_or_default().trim().is_empty() {
                return Err(ActionError::Conflict(
                    "a reason is required for reopen-task".to_owned(),
                ));
            }
        }
        Action::Work => {
            let parent = card
                .parent
                .as_deref()
                .ok_or_else(|| ActionError::Conflict("task has no parent".to_owned()))?;
            let parent = session.require(parent)?;
            if parent.state != State::Doing {
                return Err(ActionError::Conflict(
                    "the parent batch is not doing".to_owned(),
                ));
            }
            if let Some(other) = session.working_task_excluding(&card.id) {
                return Err(ActionError::Conflict(format!(
                    "another task is already working: {}",
                    other.id
                )));
            }
            for dependency in &card.depends_on {
                let dependency = session.require(dependency)?;
                if !matches!(dependency.state, State::Done | State::Obsolete) {
                    return Err(ActionError::Conflict(format!(
                        "dependency {} is not done or obsolete",
                        dependency.id
                    )));
                }
            }
        }
        Action::Done => {
            if commit.unwrap_or_default().trim().is_empty() {
                return Err(ActionError::Conflict(
                    "done requires the task commit".to_owned(),
                ));
            }
        }
        Action::Complete => {
            if session
                .loaded
                .accepted_review_for(&card.id, &card.intent_owner)
                .is_none()
            {
                return Err(ActionError::Conflict(
                    "an independent accepted review is required".to_owned(),
                ));
            }
        }
        Action::Cancel | Action::Block if reason.unwrap_or_default().trim().is_empty() => {
            return Err(ActionError::Conflict(
                "a reason is required for this action".to_owned(),
            ));
        }
        _ => {}
    }
    Ok(())
}

fn cascade_cancel(session: &mut Session, batch: &ProcessCard) -> Result<()> {
    if batch.kind != Kind::Batch {
        return Ok(());
    }
    let children: Vec<String> = session
        .child_tasks(&batch.id)
        .into_iter()
        .filter(|task| task.state != State::Canceled)
        .map(|task| task.id.clone())
        .collect();
    for id in children {
        let mut task = session
            .loaded
            .find(&id)
            .cloned()
            .ok_or_else(|| ActionError::NotFound(id.clone()))?;
        let previous = task.state;
        task.state = State::Canceled;
        task.validate()?;
        session.loaded.save_card(&mut task)?;
        let event = Event {
            id: session.loaded.next_event_id("cancel", Some(&task.id)),
            at: now(),
            action: "cancel".to_owned(),
            card: Some(task.id.clone()),
            from: Some(previous),
            to: Some(State::Canceled),
            actor: session.actor.clone(),
            executor: session.actor.clone(),
            authorised_by: None,
            authorization: None,
            reason: Some(format!("parent {} canceled", batch.id)),
            commit: None,
            metadata: Default::default(),
        };
        session.loaded.write_event(&event)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn estimate(
    root: &Path,
    actor: &str,
    id: &str,
    tokens: (f64, f64, f64),
    agent_hours: (f64, f64, f64),
    reestimate: bool,
    reason: Option<&str>,
    authorised_by: Option<&str>,
    authorization: Option<&str>,
) -> Result<ProcessCard> {
    let mut session = Session::load(root, actor)?;
    if session.is_agent() {
        if reestimate {
            let authorised_by = authorised_by.ok_or_else(|| {
                ActionError::Forbidden("re-estimation needs an owner authoriser".to_owned())
            })?;
            if authorized_human(&session, authorised_by, authorization, true)? {
                // okay
            }
        }
    } else if !session.is_owner() {
        return Err(ActionError::Forbidden(
            "only the owner or the agent can estimate".to_owned(),
        ));
    }
    let card = session.require(id)?.clone();
    if card.kind != Kind::Batch {
        return Err(ActionError::BadRequest(
            "only batches carry estimates".to_owned(),
        ));
    }
    let mut next = card.clone();
    next.set_estimate(mdboard_core::process::Estimate::from_parts(
        tokens,
        agent_hours,
    ));
    next.validate()?;
    session.loaded.save_card(&mut next)?;
    let event = Event {
        id: session
            .loaded
            .next_event_id(if reestimate { "reestimate" } else { "estimate" }, Some(id)),
        at: now(),
        action: if reestimate { "reestimate" } else { "estimate" }.to_owned(),
        card: Some(next.id.clone()),
        from: Some(next.state),
        to: Some(next.state),
        actor: actor.to_owned(),
        executor: actor.to_owned(),
        authorised_by: authorised_by.map(str::to_owned),
        authorization: authorization.map(str::to_owned),
        reason: reason.map(str::to_owned),
        commit: None,
        metadata: Default::default(),
    };
    session.loaded.write_event(&event)?;
    Ok(next)
}

fn authorized_human(
    session: &Session,
    authorised_by: &str,
    authorization: Option<&str>,
    owner_only: bool,
) -> Result<bool> {
    if authorization.unwrap_or_default().trim().is_empty() {
        return Err(ActionError::Forbidden(
            "the human's authorisation statement is required".to_owned(),
        ));
    }
    let actors = Actors::load_optional(&session.loaded.process_root().join("actors.toml"))?;
    let actor = actors
        .get(authorised_by)
        .ok_or_else(|| ActionError::NotFound(format!("actor `{authorised_by}` not found")))?;
    if actor.kind != ActorKind::Human {
        return Err(ActionError::Forbidden(
            "the authoriser must be human".to_owned(),
        ));
    }
    if owner_only && !actor.has_role("owner") {
        return Err(ActionError::Forbidden(
            "the owner must authorise this action".to_owned(),
        ));
    }
    Ok(true)
}

pub fn task_new(
    root: &Path,
    actor: &str,
    parent: &str,
    title: &str,
    body: &str,
    depends_on: &[String],
) -> Result<ProcessCard> {
    let mut session = Session::load(root, actor)?;
    if !session.is_agent() && !session.is_human() {
        return Err(ActionError::Forbidden("not allowed".to_owned()));
    }
    let parent_card = session.require(parent)?.clone();
    if parent_card.kind != Kind::Batch || parent_card.state != State::Breakdown {
        return Err(ActionError::Conflict(
            "tasks can only be created during breakdown".to_owned(),
        ));
    }
    let id = session.loaded.next_task_id(parent);
    let mut task = ProcessCard::new_task(id, parent, title, &parent_card.intent_owner, now());
    task.created_by = actor.to_owned();
    task.depends_on = depends_on.to_vec();
    if !body.trim().is_empty() {
        task.body = body.trim().to_owned();
    }
    task.validate()?;
    session.loaded.create_card(task.clone())?;
    let event = Event {
        id: session.loaded.next_event_id("task-new", Some(&task.id)),
        at: now(),
        action: "task-new".to_owned(),
        card: Some(task.id.clone()),
        from: None,
        to: Some(task.state),
        actor: actor.to_owned(),
        executor: actor.to_owned(),
        authorised_by: None,
        authorization: None,
        reason: None,
        commit: None,
        metadata: Default::default(),
    };
    session.loaded.write_event(&event)?;
    Ok(task)
}

#[allow(clippy::too_many_arguments)]
fn today() -> String {
    chrono::Utc::now().date_naive().to_string()
}

fn vote_authority(
    session: &Session,
    authorised_by: Option<&str>,
    authorization: Option<&str>,
) -> Result<()> {
    check_authority(session, Authority::Human, authorised_by, authorization)
}

fn latest_open_vote(session: &Session, card: &str) -> Result<Vote> {
    session
        .loaded
        .latest_vote_for(card)
        .filter(|vote| vote.status == VoteStatus::Open)
        .cloned()
        .ok_or_else(|| ActionError::Conflict(format!("no open vote for {card}")))
}

fn sync_expired(vote: &mut Vote, stakeholders: &StakeholderConfig) {
    let now = today();
    for member in &stakeholders.members {
        if now.as_str() > member.seat_until.as_str() {
            if let Some(existing) = vote
                .votes
                .iter_mut()
                .find(|item| item.email.eq_ignore_ascii_case(&member.email))
            {
                existing.choice = VoteChoice::Expired;
                existing.recorded_by = Some("system".to_owned());
                existing.recorded_at = Some(now.clone());
            } else {
                vote.votes.push(MemberVote {
                    name: member.name.clone(),
                    email: member.email.clone(),
                    seats: member.seats,
                    choice: VoteChoice::Expired,
                    recorded_by: Some("system".to_owned()),
                    recorded_at: Some(now.clone()),
                });
            }
        }
    }
    vote.allocated_seats = stakeholders.total_seats();
}

fn write_vote(session: &mut Session, vote: &Vote, actor: &str) -> Result<()> {
    session.loaded.write_vote(vote)?;
    let event = Event {
        id: session.loaded.next_event_id("vote", Some(&vote.card)),
        at: now(),
        action: "vote".to_owned(),
        card: Some(vote.card.clone()),
        from: None,
        to: None,
        actor: actor.to_owned(),
        executor: actor.to_owned(),
        authorised_by: None,
        authorization: None,
        reason: Some(format!("{}: {:?}", vote.id, vote.status)),
        commit: None,
        metadata: Default::default(),
    };
    session.loaded.write_event(&event)?;
    Ok(())
}

pub fn open_vote(
    root: &Path,
    actor: &str,
    card_id: &str,
    authorised_by: Option<&str>,
    authorization: Option<&str>,
) -> Result<Vote> {
    let mut session = Session::load(root, actor)?;
    let card = session.require(card_id)?.clone();
    if !card.kind.is_intent() || card.state != State::Pitch {
        return Err(ActionError::Conflict(
            "a vote can only be opened for a pitched card".to_owned(),
        ));
    }
    vote_authority(&session, authorised_by, authorization)?;
    if session
        .loaded
        .latest_vote_for(card_id)
        .is_some_and(|vote| vote.status == VoteStatus::Open)
    {
        return Err(ActionError::Conflict(format!(
            "a vote is already open for {card_id}"
        )));
    }
    let mut vote = Vote {
        id: session.loaded.next_vote_id(card_id),
        card: card.id.clone(),
        opened_by: actor.to_owned(),
        opened_at: now(),
        closed_at: None,
        status: VoteStatus::Open,
        quorum_seats: session.loaded.policy.stakeholders.quorum_seats,
        allocated_seats: session.loaded.policy.stakeholders.total_seats(),
        counted_seats: 0,
        votes: Vec::new(),
        veto: false,
        veto_by: None,
        veto_reason: None,
        tie_break: None,
        decision: None,
        reason: None,
    };
    sync_expired(&mut vote, &session.loaded.policy.stakeholders);
    session.loaded.write_vote(&vote)?;
    Ok(vote)
}

pub fn record_vote(
    root: &Path,
    actor: &str,
    card_id: &str,
    email: &str,
    choice: VoteChoice,
) -> Result<Vote> {
    let mut session = Session::load(root, actor)?;
    let mut vote = latest_open_vote(&session, card_id)?;
    sync_expired(&mut vote, &session.loaded.policy.stakeholders);
    let member = session
        .loaded
        .policy
        .stakeholders
        .members
        .iter()
        .find(|member| member.email.eq_ignore_ascii_case(email))
        .ok_or_else(|| ActionError::NotFound(format!("stakeholder `{email}` not found")))?;
    if today() > member.seat_until {
        return Err(ActionError::Conflict(format!(
            "stakeholder `{email}` has an expired seat"
        )));
    }
    let entry = MemberVote {
        name: member.name.clone(),
        email: member.email.clone(),
        seats: member.seats,
        choice,
        recorded_by: Some(actor.to_owned()),
        recorded_at: Some(now()),
    };
    if let Some(existing) = vote
        .votes
        .iter_mut()
        .find(|item| item.email.eq_ignore_ascii_case(email))
    {
        *existing = entry;
    } else {
        vote.votes.push(entry);
    }
    write_vote(&mut session, &vote, actor)?;
    Ok(vote)
}

pub fn veto_vote(
    root: &Path,
    actor: &str,
    card_id: &str,
    reason: &str,
    authorised_by: Option<&str>,
    authorization: Option<&str>,
) -> Result<Vote> {
    if reason.trim().is_empty() {
        return Err(ActionError::Conflict(
            "a veto reason is required".to_owned(),
        ));
    }
    let mut session = Session::load(root, actor)?;
    vote_authority(&session, authorised_by, authorization)?;
    let mut vote = latest_open_vote(&session, card_id)?;
    vote.veto = true;
    vote.veto_by = Some(actor.to_owned());
    vote.veto_reason = Some(reason.to_owned());
    vote.status = VoteStatus::Vetoed;
    vote.closed_at = Some(now());
    vote.reason = Some(reason.to_owned());
    write_vote(&mut session, &vote, actor)?;
    Ok(vote)
}

pub fn resolve_vote(
    root: &Path,
    actor: &str,
    card_id: &str,
    tie_break: Option<VoteChoice>,
) -> Result<Vote> {
    let mut session = Session::load(root, actor)?;
    let mut vote = latest_open_vote(&session, card_id)?;
    sync_expired(&mut vote, &session.loaded.policy.stakeholders);
    let members = session.loaded.policy.stakeholders.members.clone();
    if members.is_empty() {
        vote.status = VoteStatus::Passed;
        vote.decision = Some("no stakeholders: unanimous approval".to_owned());
        vote.closed_at = Some(now());
    } else {
        let missing: Vec<_> = members
            .iter()
            .filter(|member| {
                !vote
                    .votes
                    .iter()
                    .any(|item| item.email.eq_ignore_ascii_case(&member.email))
            })
            .map(|member| member.email.clone())
            .collect();
        let expired: u32 = vote
            .votes
            .iter()
            .filter(|item| item.choice == VoteChoice::Expired)
            .map(|item| item.seats)
            .sum();
        let counted: u32 = vote
            .votes
            .iter()
            .filter(|item| {
                matches!(
                    item.choice,
                    VoteChoice::Approve | VoteChoice::Reject | VoteChoice::Abstain
                )
            })
            .map(|item| item.seats)
            .sum();
        vote.allocated_seats = members.iter().map(|member| member.seats).sum();
        vote.counted_seats = counted;
        let available = vote.allocated_seats.saturating_sub(expired);
        let effective_quorum = vote.quorum_seats.min(available);
        if vote.veto {
            vote.status = VoteStatus::Vetoed;
        } else if available == 0 {
            vote.status = VoteStatus::Passed;
            vote.decision = Some("all seats expired: automatic approval".to_owned());
        } else if !missing.is_empty() {
            vote.status = VoteStatus::Unresolved;
            vote.reason = Some(format!("missing results: {}", missing.join(", ")));
        } else if counted < effective_quorum {
            vote.status = VoteStatus::Unresolved;
            vote.reason = Some(format!(
                "quorum not met: {counted} of {effective_quorum} seats"
            ));
        } else {
            let approve: u32 = vote
                .votes
                .iter()
                .filter(|item| item.choice == VoteChoice::Approve)
                .map(|item| item.seats)
                .sum();
            let reject: u32 = vote
                .votes
                .iter()
                .filter(|item| item.choice == VoteChoice::Reject)
                .map(|item| item.seats)
                .sum();
            if approve * 2 > available {
                vote.status = VoteStatus::Passed;
                vote.decision = Some("seat majority approved".to_owned());
            } else if reject * 2 > available {
                vote.status = VoteStatus::Rejected;
                vote.decision = Some("seat majority rejected".to_owned());
            } else if let Some(choice) = tie_break {
                vote.tie_break = Some(choice);
                vote.status = match choice {
                    VoteChoice::Approve => VoteStatus::Passed,
                    VoteChoice::Reject => VoteStatus::Rejected,
                    _ => VoteStatus::Unresolved,
                };
                vote.decision = Some(format!("guardian tie-break: {choice:?}"));
            } else {
                vote.status = VoteStatus::Unresolved;
                vote.reason = Some("tie or no majority; guardian tie-break required".to_owned());
            }
        }
        vote.closed_at = Some(now());
    }
    write_vote(&mut session, &vote, actor)?;
    Ok(vote)
}

pub fn close_vote_unresolved(
    root: &Path,
    actor: &str,
    card_id: &str,
    reason: &str,
    authorised_by: Option<&str>,
    authorization: Option<&str>,
) -> Result<Vote> {
    let mut session = Session::load(root, actor)?;
    vote_authority(&session, authorised_by, authorization)?;
    let mut vote = latest_open_vote(&session, card_id)?;
    vote.status = VoteStatus::Unresolved;
    vote.closed_at = Some(now());
    vote.reason = Some(reason.to_owned());
    write_vote(&mut session, &vote, actor)?;
    Ok(vote)
}

#[allow(clippy::too_many_arguments)]
fn write_order_event(
    session: &mut Session,
    order: &Order,
    action: &str,
    actor: &str,
    reason: Option<&str>,
) -> Result<()> {
    let event = Event {
        id: session.loaded.next_event_id(action, None),
        at: now(),
        action: action.to_owned(),
        card: None,
        from: None,
        to: None,
        actor: actor.to_owned(),
        executor: actor.to_owned(),
        authorised_by: Some(order.authorised_by.clone()),
        authorization: order.authorisation.clone(),
        reason: reason.map(str::to_owned),
        commit: None,
        metadata: Default::default(),
    };
    session.loaded.write_event(&event)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn propose_order(
    root: &Path,
    actor: &str,
    title: &str,
    instruction: &str,
    scope: OrderScope,
    rationale: &str,
    consequences: &str,
    authorised_by: Option<&str>,
    authorization: Option<&str>,
) -> Result<Order> {
    let mut session = Session::load(root, actor)?;
    check_authority(&session, Authority::Owner, authorised_by, authorization)?;
    let id = session.loaded.next_order_id();
    let at = now();
    let order = Order {
        id,
        title: title.to_owned(),
        status: OrderStatus::Draft,
        scope,
        instruction: instruction.to_owned(),
        rationale: rationale.to_owned(),
        consequences: consequences.to_owned(),
        authorised_by: authorised_by.unwrap_or(actor).to_owned(),
        authorisation: authorization.map(str::to_owned),
        created_at: at.clone(),
        updated_at: at,
        history: Vec::new(),
    };
    order.validate()?;
    session.loaded.write_order(&order)?;
    write_order_event(&mut session, &order, "order-propose", actor, None)?;
    Ok(order)
}

pub fn add_order(
    root: &Path,
    actor: &str,
    id: &str,
    authorised_by: Option<&str>,
    authorization: Option<&str>,
) -> Result<Order> {
    let mut session = Session::load(root, actor)?;
    check_authority(&session, Authority::Owner, authorised_by, authorization)?;
    let mut order = session
        .loaded
        .orders
        .iter()
        .find(|order| order.id == id)
        .cloned()
        .ok_or_else(|| ActionError::NotFound(format!("order `{id}` not found")))?;
    if order.status != OrderStatus::Draft {
        return Err(ActionError::Conflict(
            "only a draft order can be added".to_owned(),
        ));
    }
    order.status = OrderStatus::Active;
    order.updated_at = now();
    order.authorised_by = authorised_by.unwrap_or(actor).to_owned();
    order.authorisation = authorization.map(str::to_owned);
    session.loaded.write_order(&order)?;
    write_order_event(&mut session, &order, "order-add", actor, None)?;
    Ok(order)
}

pub fn amend_order(
    root: &Path,
    actor: &str,
    id: &str,
    instruction: &str,
    reason: &str,
    authorised_by: Option<&str>,
    authorization: Option<&str>,
) -> Result<Order> {
    if instruction.trim().is_empty() || reason.trim().is_empty() {
        return Err(ActionError::Conflict(
            "amendment instruction and reason are required".to_owned(),
        ));
    }
    let mut session = Session::load(root, actor)?;
    check_authority(&session, Authority::Owner, authorised_by, authorization)?;
    let mut order = session
        .loaded
        .orders
        .iter()
        .find(|order| order.id == id)
        .cloned()
        .ok_or_else(|| ActionError::NotFound(format!("order `{id}` not found")))?;
    if order.status != OrderStatus::Active {
        return Err(ActionError::Conflict(
            "only an active order can be amended".to_owned(),
        ));
    }
    let at = now();
    order.history.push(OrderVersion {
        at: at.clone(),
        by: actor.to_owned(),
        instruction: order.instruction.clone(),
        status: order.status,
        reason: Some(reason.to_owned()),
    });
    order.instruction = instruction.to_owned();
    order.updated_at = at;
    order.authorised_by = authorised_by.unwrap_or(actor).to_owned();
    order.authorisation = authorization.map(str::to_owned);
    session.loaded.write_order(&order)?;
    write_order_event(&mut session, &order, "order-amend", actor, Some(reason))?;
    Ok(order)
}

pub fn retire_order(
    root: &Path,
    actor: &str,
    id: &str,
    reason: &str,
    authorised_by: Option<&str>,
    authorization: Option<&str>,
) -> Result<Order> {
    if reason.trim().is_empty() {
        return Err(ActionError::Conflict(
            "a retirement reason is required".to_owned(),
        ));
    }
    let mut session = Session::load(root, actor)?;
    check_authority(&session, Authority::Owner, authorised_by, authorization)?;
    let mut order = session
        .loaded
        .orders
        .iter()
        .find(|order| order.id == id)
        .cloned()
        .ok_or_else(|| ActionError::NotFound(format!("order `{id}` not found")))?;
    if order.status == OrderStatus::Retired {
        return Err(ActionError::Conflict(format!(
            "order `{id}` is already retired"
        )));
    }
    let at = now();
    order.history.push(OrderVersion {
        at: at.clone(),
        by: actor.to_owned(),
        instruction: order.instruction.clone(),
        status: order.status,
        reason: Some(reason.to_owned()),
    });
    order.status = OrderStatus::Retired;
    order.updated_at = at;
    order.authorised_by = authorised_by.unwrap_or(actor).to_owned();
    order.authorisation = authorization.map(str::to_owned);
    session.loaded.write_order(&order)?;
    write_order_event(&mut session, &order, "order-retire", actor, Some(reason))?;
    Ok(order)
}

#[must_use]
pub fn has_ssh_keys(loaded: &ProcessLoaded) -> bool {
    loaded.policy.guardian.ssh_key.is_some()
        || loaded
            .policy
            .guardian
            .co_guardians
            .iter()
            .any(|co| co.ssh_key.is_some())
}

fn ssh_key_for<'a>(loaded: &'a ProcessLoaded, email: &str) -> Option<(&'a str, Option<&'a str>)> {
    if loaded.policy.guardian.email.eq_ignore_ascii_case(email) {
        return loaded
            .policy
            .guardian
            .ssh_key
            .as_deref()
            .map(|key| (key, loaded.policy.guardian.ssh_fingerprint.as_deref()));
    }
    loaded
        .policy
        .guardian
        .co_guardians
        .iter()
        .find(|co| co.email.eq_ignore_ascii_case(email))
        .and_then(|co| {
            co.ssh_key
                .as_deref()
                .map(|key| (key, co.ssh_fingerprint.as_deref()))
        })
}

pub fn ssh_challenge(root: &Path, actor: &str, card_id: &str) -> Result<String> {
    let mut session = Session::load(root, actor)?;
    session.require(card_id)?;
    let nonce = format!("{}-{}", now(), session.loaded.events.len());
    let event = Event {
        id: session.loaded.next_event_id("ssh-challenge", Some(card_id)),
        at: now(),
        action: "ssh-challenge".to_owned(),
        card: Some(card_id.to_owned()),
        from: None,
        to: None,
        actor: actor.to_owned(),
        executor: actor.to_owned(),
        authorised_by: None,
        authorization: None,
        reason: Some(nonce.clone()),
        commit: None,
        metadata: Default::default(),
    };
    session.loaded.write_event(&event)?;
    Ok(nonce)
}

pub fn verify_ssh_signature(
    root: &Path,
    actor: &str,
    card_id: &str,
    nonce: &str,
    signature: &Path,
    email: &str,
) -> Result<()> {
    let mut session = Session::load(root, actor)?;
    session.require(card_id)?;
    let challenge = session
        .loaded
        .events
        .iter()
        .find(|event| {
            event.action == "ssh-challenge"
                && event.card.as_deref() == Some(card_id)
                && event.reason.as_deref() == Some(nonce)
        })
        .ok_or_else(|| ActionError::NotFound("SSH challenge not found".to_owned()))?;
    if session.loaded.events.iter().any(|event| {
        event.action == "ssh-verified"
            && event.card.as_deref() == Some(card_id)
            && event.metadata.get("nonce").map(String::as_str) == Some(nonce)
    }) {
        return Err(ActionError::Conflict(
            "SSH challenge already used".to_owned(),
        ));
    }
    let (public_key, fingerprint) = ssh_key_for(&session.loaded, email)
        .ok_or_else(|| ActionError::Forbidden(format!("no SSH key configured for `{email}`")))?;
    let dir = std::env::temp_dir().join(format!("mdboard-ssh-{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let nonce_path = dir.join("nonce");
    let allowed_path = dir.join("allowed_signers");
    fs::write(&nonce_path, nonce)?;
    fs::write(&allowed_path, format!("{email} {public_key}\n"))?;
    let output = Command::new("ssh-keygen")
        .args(["-Y", "verify", "-f"])
        .arg(&allowed_path)
        .args(["-I", email, "-n", "mdboard", "-s"])
        .arg(signature)
        .stdin(Stdio::from(fs::File::open(&nonce_path)?))
        .output();
    let _ = fs::remove_dir_all(&dir);
    let output =
        output.map_err(|error| ActionError::Internal(format!("ssh-keygen failed: {error}")))?;
    if !output.status.success() {
        return Err(ActionError::Forbidden(format!(
            "SSH signature verification failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert("nonce".to_owned(), nonce.to_owned());
    metadata.insert("email".to_owned(), email.to_owned());
    if let Some(fingerprint) = fingerprint {
        metadata.insert("fingerprint".to_owned(), fingerprint.to_owned());
    }
    let event = Event {
        id: session.loaded.next_event_id("ssh-verified", Some(card_id)),
        at: now(),
        action: "ssh-verified".to_owned(),
        card: Some(card_id.to_owned()),
        from: None,
        to: None,
        actor: actor.to_owned(),
        executor: actor.to_owned(),
        authorised_by: Some(email.to_owned()),
        authorization: Some("SSH challenge signature".to_owned()),
        reason: Some(format!("verified {nonce} ({})", challenge.id)),
        commit: None,
        metadata,
    };
    session.loaded.write_event(&event)?;
    Ok(())
}

pub fn override_quality_gate(root: &Path, actor: &str, card_id: &str, reason: &str) -> Result<()> {
    if reason.trim().is_empty() {
        return Err(ActionError::Conflict(
            "an override reason is required".to_owned(),
        ));
    }
    let mut session = Session::load(root, actor)?;
    if !session.is_agent() {
        return Err(ActionError::Forbidden(
            "only the agent records the override".to_owned(),
        ));
    }
    let card = session.require(card_id)?.clone();
    if !session.loaded.policy.quality_gate.require_override_reason {
        // The reason is still recorded; the policy only controls whether callers must supply one.
    }
    let event = Event {
        id: session
            .loaded
            .next_event_id("quality-gate-overridden", Some(card_id)),
        at: now(),
        action: "quality-gate-overridden".to_owned(),
        card: Some(card_id.to_owned()),
        from: Some(card.state),
        to: Some(card.state),
        actor: actor.to_owned(),
        executor: actor.to_owned(),
        authorised_by: Some("owner".to_owned()),
        authorization: Some(reason.to_owned()),
        reason: Some(reason.to_owned()),
        commit: None,
        metadata: Default::default(),
    };
    session.loaded.write_event(&event)?;
    Ok(())
}

pub fn archive_card(
    root: &Path,
    actor: &str,
    id: &str,
    reason: Option<&str>,
    authorised_by: Option<&str>,
    authorization: Option<&str>,
) -> Result<ProcessCard> {
    let mut session = Session::load(root, actor)?;
    check_authority(&session, Authority::Owner, authorised_by, authorization)?;
    let mut card = session.require(id)?.clone();
    if card.archived_at.is_some() {
        return Err(ActionError::Conflict(format!("{id} is already archived")));
    }
    if card.kind == Kind::Batch
        && matches!(
            card.state,
            State::Roadmap | State::Breakdown | State::Doing | State::Handover
        )
    {
        return Err(ActionError::Conflict(
            "active or roadmap batches must be resolved before archiving".to_owned(),
        ));
    }
    if card.kind == Kind::Task && card.state == State::Working {
        return Err(ActionError::Conflict(
            "a working task cannot be archived".to_owned(),
        ));
    }
    if session
        .child_tasks(&card.id)
        .iter()
        .any(|task| !matches!(task.state, State::Done | State::Canceled | State::Error))
    {
        return Err(ActionError::Conflict(
            "a card with active children cannot be archived".to_owned(),
        ));
    }
    card.archived_at = Some(now());
    card.archived_by = Some(actor.to_owned());
    card.archive_reason = reason.map(str::to_owned);
    card.updated_at = now();
    session.loaded.save_card(&mut card)?;
    let event = Event {
        id: session.loaded.next_event_id("archive", Some(id)),
        at: now(),
        action: "archive".to_owned(),
        card: Some(id.to_owned()),
        from: Some(card.state),
        to: Some(card.state),
        actor: actor.to_owned(),
        executor: actor.to_owned(),
        authorised_by: authorised_by.map(str::to_owned),
        authorization: authorization.map(str::to_owned),
        reason: reason.map(str::to_owned),
        commit: None,
        metadata: Default::default(),
    };
    session.loaded.write_event(&event)?;
    Ok(card)
}

pub fn unarchive_card(
    root: &Path,
    actor: &str,
    id: &str,
    authorised_by: Option<&str>,
    authorization: Option<&str>,
) -> Result<ProcessCard> {
    let mut session = Session::load(root, actor)?;
    check_authority(&session, Authority::Owner, authorised_by, authorization)?;
    let mut card = session.require(id)?.clone();
    if card.archived_at.is_none() {
        return Err(ActionError::Conflict(format!("{id} is not archived")));
    }
    card.archived_at = None;
    card.archived_by = None;
    card.archive_reason = None;
    card.updated_at = now();
    session.loaded.save_card(&mut card)?;
    let event = Event {
        id: session.loaded.next_event_id("unarchive", Some(id)),
        at: now(),
        action: "unarchive".to_owned(),
        card: Some(id.to_owned()),
        from: Some(card.state),
        to: Some(card.state),
        actor: actor.to_owned(),
        executor: actor.to_owned(),
        authorised_by: authorised_by.map(str::to_owned),
        authorization: authorization.map(str::to_owned),
        reason: None,
        commit: None,
        metadata: Default::default(),
    };
    session.loaded.write_event(&event)?;
    Ok(card)
}

pub fn record_quality_gate(
    root: &Path,
    actor: &str,
    card_id: &str,
    passed: bool,
    findings: &[String],
) -> Result<()> {
    let mut session = Session::load(root, actor)?;
    if !session.is_agent() {
        return Err(ActionError::Forbidden(
            "the quality gate is run by the agent".to_owned(),
        ));
    }
    let card = session.require(card_id)?.clone();
    if !card.kind.is_intent() || !matches!(card.state, State::Draft | State::Refine) {
        return Err(ActionError::Conflict(
            "the quality gate runs on a draft or refine intent card".to_owned(),
        ));
    }
    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert(
        "result".to_owned(),
        if passed {
            "passed".to_owned()
        } else {
            "failed".to_owned()
        },
    );
    metadata.insert("findings".to_owned(), findings.len().to_string());
    if !findings.is_empty() {
        let dir = session.loaded.attachments_dir(card_id);
        std::fs::create_dir_all(&dir)?;
        let text = findings.join("\n\n");
        std::fs::write(dir.join("quality-gate.md"), text)?;
        metadata.insert("attachment".to_owned(), "quality-gate.md".to_owned());
    }
    let action = if passed {
        "quality-gate-passed"
    } else {
        "quality-gate-failed"
    };
    let event = Event {
        id: session.loaded.next_event_id(action, Some(card_id)),
        at: now(),
        action: action.to_owned(),
        card: Some(card_id.to_owned()),
        from: Some(card.state),
        to: Some(card.state),
        actor: actor.to_owned(),
        executor: actor.to_owned(),
        authorised_by: None,
        authorization: None,
        reason: findings.first().cloned(),
        commit: None,
        metadata,
    };
    session.loaded.write_event(&event)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn review(
    root: &Path,
    actor: &str,
    batch: &str,
    reviewer: &str,
    authored_by: &str,
    verdict: Verdict,
    findings: &[(String, String)],
    round: u32,
) -> Result<ReviewArtifact> {
    let mut session = Session::load(root, actor)?;
    let card = session.require(batch)?.clone();
    if card.kind != Kind::Batch || card.state != State::Handover {
        return Err(ActionError::Conflict(
            "reviews are recorded while the batch is in handover".to_owned(),
        ));
    }
    if reviewer == card.intent_owner {
        return Err(ActionError::Forbidden(
            "the reviewer must differ from the intent owner".to_owned(),
        ));
    }
    let id = session.loaded.next_review_id(batch, round);
    let artifact = ReviewArtifact {
        id,
        batch: batch.to_owned(),
        round,
        reviewer: reviewer.to_owned(),
        authored_by: authored_by.to_owned(),
        transcribed_by: (actor != authored_by).then(|| actor.to_owned()),
        verdict,
        findings: findings
            .iter()
            .map(|(id, text)| Finding {
                id: id.clone(),
                text: text.clone(),
            })
            .collect(),
        created_at: now(),
    };
    artifact.validate()?;
    session.loaded.write_review(&artifact)?;
    let event = Event {
        id: session.loaded.next_event_id("review", Some(batch)),
        at: now(),
        action: "review".to_owned(),
        card: Some(batch.to_owned()),
        from: Some(card.state),
        to: Some(card.state),
        actor: actor.to_owned(),
        executor: actor.to_owned(),
        authorised_by: None,
        authorization: None,
        reason: Some(format!("review {}: {:?}", artifact.id, artifact.verdict)),
        commit: None,
        metadata: Default::default(),
    };
    session.loaded.write_event(&event)?;
    Ok(artifact)
}

fn create_rework_tasks(session: &mut Session, batch: &ProcessCard) -> Result<()> {
    let latest = session
        .loaded
        .reviews
        .iter()
        .filter(|review| review.batch == batch.id && review.verdict == Verdict::ChangesRequired)
        .max_by_key(|review| review.round)
        .cloned();
    let Some(review) = latest else {
        return Ok(());
    };
    for finding in &review.findings {
        let id = session.loaded.next_task_id(&batch.id);
        let mut task = ProcessCard::new_task(
            id,
            &batch.id,
            format!("Rework: {}", finding.text),
            &batch.intent_owner,
            now(),
        );
        task.created_by = session.actor.clone();
        task.source_review = Some(review.id.clone());
        task.body = format!("## Finding\n\n{}: {}\n", finding.id, finding.text);
        task.validate()?;
        session.loaded.create_card(task)?;
    }
    Ok(())
}

pub fn meeting_open(
    root: &Path,
    actor: &str,
    kind: MeetingKind,
    batch: &str,
    trigger: &str,
) -> Result<Meeting> {
    let mut session = Session::load(root, actor)?;
    let card = session.require(batch)?.clone();
    if card.kind != Kind::Batch {
        return Err(ActionError::BadRequest(
            "meetings link to batches".to_owned(),
        ));
    }
    let id = session.loaded.next_meeting_id(batch);
    let meeting = Meeting {
        id,
        kind,
        status: MeetingStatus::Pending,
        trigger: trigger.to_owned(),
        due: None,
        linked_cards: vec![batch.to_owned()],
        questions: Vec::new(),
        decision: None,
        attendees: Vec::new(),
        created_at: now(),
        updated_at: now(),
    };
    meeting.validate()?;
    session.loaded.write_meeting(&meeting)?;
    Ok(meeting)
}

pub fn meeting_question(root: &Path, actor: &str, meeting_id: &str, text: &str) -> Result<Meeting> {
    let mut session = Session::load(root, actor)?;
    let mut meeting = session
        .loaded
        .meetings
        .iter()
        .find(|meeting| meeting.id == meeting_id)
        .cloned()
        .ok_or_else(|| ActionError::NotFound(meeting_id.to_owned()))?;
    let seq = meeting.questions.len() + 1;
    meeting.questions.push(Question {
        id: format!("Q{seq}"),
        text: text.to_owned(),
        resolved: false,
    });
    meeting.updated_at = now();
    meeting.validate()?;
    session.loaded.write_meeting(&meeting)?;
    Ok(meeting)
}

pub fn meeting_resolve(
    root: &Path,
    actor: &str,
    meeting_id: &str,
    question_id: &str,
) -> Result<Meeting> {
    let mut session = Session::load(root, actor)?;
    let mut meeting = session
        .loaded
        .meetings
        .iter()
        .find(|meeting| meeting.id == meeting_id)
        .cloned()
        .ok_or_else(|| ActionError::NotFound(meeting_id.to_owned()))?;
    let question = meeting
        .questions
        .iter_mut()
        .find(|question| question.id == question_id)
        .ok_or_else(|| ActionError::NotFound(question_id.to_owned()))?;
    question.resolved = true;
    meeting.updated_at = now();
    meeting.validate()?;
    session.loaded.write_meeting(&meeting)?;
    Ok(meeting)
}

pub fn meeting_accept(root: &Path, actor: &str, meeting_id: &str) -> Result<Meeting> {
    let mut session = Session::load(root, actor)?;
    if !session.is_human() {
        return Err(ActionError::Forbidden(
            "a human accepts a meeting gate".to_owned(),
        ));
    }
    let mut meeting = session
        .loaded
        .meetings
        .iter()
        .find(|meeting| meeting.id == meeting_id)
        .cloned()
        .ok_or_else(|| ActionError::NotFound(meeting_id.to_owned()))?;
    if meeting.questions.iter().any(|question| !question.resolved) {
        return Err(ActionError::Conflict(
            "resolve the meeting questions before accepting".to_owned(),
        ));
    }
    meeting.status = MeetingStatus::Accepted;
    meeting.decision = Some("accepted".to_owned());
    meeting.updated_at = now();
    meeting.validate()?;
    session.loaded.write_meeting(&meeting)?;
    Ok(meeting)
}

pub fn meeting_defer(root: &Path, actor: &str, meeting_id: &str, until: &str) -> Result<Meeting> {
    let mut session = Session::load(root, actor)?;
    let mut meeting = session
        .loaded
        .meetings
        .iter()
        .find(|meeting| meeting.id == meeting_id)
        .cloned()
        .ok_or_else(|| ActionError::NotFound(meeting_id.to_owned()))?;
    meeting.status = MeetingStatus::Deferred;
    meeting.due = Some(until.to_owned());
    meeting.updated_at = now();
    meeting.validate()?;
    session.loaded.write_meeting(&meeting)?;
    Ok(meeting)
}

pub fn meeting_dismiss(
    root: &Path,
    actor: &str,
    meeting_id: &str,
    reason: &str,
) -> Result<Meeting> {
    if reason.trim().is_empty() {
        return Err(ActionError::BadRequest("a reason is required".to_owned()));
    }
    let mut session = Session::load(root, actor)?;
    let mut meeting = session
        .loaded
        .meetings
        .iter()
        .find(|meeting| meeting.id == meeting_id)
        .cloned()
        .ok_or_else(|| ActionError::NotFound(meeting_id.to_owned()))?;
    meeting.status = MeetingStatus::Dismissed;
    meeting.decision = Some(reason.to_owned());
    meeting.updated_at = now();
    meeting.validate()?;
    session.loaded.write_meeting(&meeting)?;
    Ok(meeting)
}

#[must_use]
pub fn legal_actions(card: &ProcessCard) -> Vec<Action> {
    card.legal_actions()
}

#[must_use]
pub fn action_authority(action: Action, kind: Kind, state: State) -> Option<Authority> {
    TRANSITIONS
        .iter()
        .find(|rule| rule.action == action && rule.from == state && rule.from_kind == kind)
        .map(|rule| rule.authority)
}

#[must_use]
pub fn all_cards(loaded: &ProcessLoaded) -> &[ProcessCard] {
    &loaded.cards
}

#[must_use]
pub fn open_meetings(loaded: &ProcessLoaded) -> Vec<&Meeting> {
    loaded
        .meetings
        .iter()
        .filter(|meeting| {
            matches!(
                meeting.status,
                MeetingStatus::Pending | MeetingStatus::Deferred
            )
        })
        .collect()
}

#[must_use]
pub fn policy(loaded: &ProcessLoaded) -> &Policy {
    &loaded.policy
}

#[must_use]
pub fn ids(loaded: &ProcessLoaded) -> BTreeSet<String> {
    loaded.cards.iter().map(|card| card.id.clone()).collect()
}

fn open_handover_meeting(session: &mut Session, batch: &ProcessCard) -> Result<()> {
    let already_open = session.loaded.meetings.iter().any(|meeting| {
        meeting.kind == MeetingKind::HandoverReview
            && meeting.linked_cards.iter().any(|id| id == &batch.id)
            && matches!(
                meeting.status,
                MeetingStatus::Pending | MeetingStatus::Deferred
            )
    });
    if already_open {
        return Ok(());
    }
    let id = session.loaded.next_meeting_id(&batch.id);
    let meeting = Meeting {
        id,
        kind: MeetingKind::HandoverReview,
        status: MeetingStatus::Pending,
        trigger: format!("{} entered handover", batch.id),
        due: None,
        linked_cards: vec![batch.id.clone()],
        questions: Vec::new(),
        decision: None,
        attendees: Vec::new(),
        created_at: now(),
        updated_at: now(),
    };
    meeting.validate()?;
    session.loaded.write_meeting(&meeting)?;
    Ok(())
}

fn close_handover_meeting(
    session: &mut Session,
    batch: &ProcessCard,
    decision: &str,
) -> Result<()> {
    let mut meetings = session
        .loaded
        .meetings
        .iter()
        .filter(|meeting| {
            meeting.kind == MeetingKind::HandoverReview
                && meeting.linked_cards.iter().any(|id| id == &batch.id)
                && matches!(
                    meeting.status,
                    MeetingStatus::Pending | MeetingStatus::Deferred
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    for meeting in &mut meetings {
        meeting.status = MeetingStatus::Accepted;
        meeting.decision = Some(decision.to_owned());
        meeting.updated_at = now();
        meeting.validate()?;
        session.loaded.write_meeting(meeting)?;
    }
    Ok(())
}

fn render_report(session: &Session, batch: &ProcessCard) -> String {
    let tasks = session.child_tasks(&batch.id);
    let reviews = session
        .loaded
        .reviews
        .iter()
        .filter(|review| review.batch == batch.id)
        .collect::<Vec<_>>();
    let mut report = String::new();
    report.push_str(&format!("# {} report\n\n", batch.id));
    report.push_str(&format!("**Batch:** {} — {}\n\n", batch.id, batch.title));
    report.push_str(&format!("**Intent owner:** {}\n\n", batch.intent_owner));
    report.push_str(batch.body.trim());
    report.push_str("\n\n## Tasks\n\n");
    for task in tasks {
        report.push_str(&format!(
            "- {} — {} ({})\n",
            task.id, task.title, task.state
        ));
    }
    report.push_str("\n## Review\n\n");
    for review in reviews {
        report.push_str(&format!(
            "- {} by {} ({:?}), round {}\n",
            review.id, review.reviewer, review.verdict, review.round
        ));
        for finding in &review.findings {
            report.push_str(&format!("  - {}: {}\n", finding.id, finding.text));
        }
    }
    report.push_str("\n## Estimate\n\n");
    if let Some(estimate) = &batch.estimate {
        report.push_str(&format!(
            "- Tokens: {:.0} weighted\n- Agent hours: {:.2} weighted\n",
            estimate.tokens.weighted, estimate.agent_hours.weighted
        ));
    }
    report
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn board() -> TempDir {
        let dir = TempDir::new().unwrap();
        for path in ["cards", "events", "meetings", "reviews", "reports"] {
            fs::create_dir_all(dir.path().join(".mdboard").join(path)).unwrap();
        }
        fs::write(
            dir.path().join("mdboard.toml"),
            r#"mdboard = "1.0.00"
schema = 3

[git]
remote = "origin"
stable = "stable"
nightly = "nightly"
upstream = "main"

[limits]
roadmap = 5

[approval]
mode = "chat"

[meetings]
consortium = "weekly"
release = "biweekly"

[costs]
subscription = "test"
eur_per_million_tokens = 0.21

[quality_gate]
enabled = false
"#,
        )
        .unwrap();
        fs::write(
            dir.path().join(".mdboard/actors.toml"),
            r#"[actors.owner]
kind = "human"
display_name = "Owner"
roles = ["owner"]

[actors.member]
kind = "human"
display_name = "Member"
roles = ["member"]

[actors.agent]
kind = "agent"
display_name = "Agent"
roles = ["agent"]
"#,
        )
        .unwrap();
        fs::write(
            dir.path().join(".mdboard/upstream.md"),
            "---\niteration: 1\nstatus: open\nbase: HEAD\nwriter: agent\nlease_expires: 2099-01-01T00:00:00Z\nstarted_at: 2026-10-24T10:00:00Z\n---\n",
        )
        .unwrap();
        dir
    }

    fn auth() -> (&'static str, &'static str) {
        ("owner", "approved")
    }

    #[test]
    fn complete_happy_path_records_report_and_changelog() {
        let dir = board();
        let root = dir.path();
        let card = spark(root, "agent", "CSV export", "", "owner").unwrap();
        let id = card.id.clone();
        let (by, statement) = auth();
        transition(
            root,
            "agent",
            &id,
            Action::Refine,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            &id,
            Action::Pitch,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            &id,
            Action::AcceptIdea,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        estimate(
            root,
            "agent",
            &id,
            (1.0, 2.0, 3.0),
            (1.0, 2.0, 3.0),
            false,
            None,
            None,
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            &id,
            Action::Roadmap,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            &id,
            Action::Breakdown,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        let task = task_new(root, "agent", &id, "Implement", "", &[]).unwrap();
        transition(
            root,
            "agent",
            &id,
            Action::Start,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            &task.id,
            Action::Work,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            &task.id,
            Action::Done,
            None,
            None,
            None,
            Some("deadbeef"),
        )
        .unwrap();
        transition(root, "agent", &id, Action::Handover, None, None, None, None).unwrap();
        review(
            root,
            "owner",
            &id,
            "member",
            "member",
            Verdict::Accepted,
            &[],
            1,
        )
        .unwrap();
        let completed =
            transition(root, "owner", &id, Action::Complete, None, None, None, None).unwrap();
        assert_eq!(completed.state, State::Completed);
        assert!(root.join(".mdboard/reports/R0001.md").exists());
        assert!(root.join("CHANGELOG.md").exists());
    }

    #[test]
    fn start_requires_an_accepted_clarification() {
        let dir = board();
        let root = dir.path();
        let card = spark(root, "agent", "Clarify me", "", "owner").unwrap();
        let id = card.id.clone();
        let (by, statement) = auth();
        transition(
            root,
            "agent",
            &id,
            Action::Refine,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            &id,
            Action::Pitch,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            &id,
            Action::AcceptIdea,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        estimate(
            root,
            "agent",
            &id,
            (1.0, 2.0, 3.0),
            (1.0, 2.0, 3.0),
            false,
            None,
            None,
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            &id,
            Action::Roadmap,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            &id,
            Action::Breakdown,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        task_new(root, "agent", &id, "Task", "", &[]).unwrap();
        let meeting =
            meeting_open(root, "agent", MeetingKind::BreakdownClarification, &id, "Q").unwrap();
        meeting_question(root, "agent", &meeting.id, "Question?").unwrap();
        let error = transition(
            root,
            "agent",
            &id,
            Action::Start,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("clarification"));
    }
    fn to_doing(root: &Path) -> String {
        let card = spark(root, "agent", "Work", "", "owner").unwrap();
        let id = card.id.clone();
        let (by, statement) = auth();
        transition(
            root,
            "agent",
            &id,
            Action::Refine,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            &id,
            Action::Pitch,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            &id,
            Action::AcceptIdea,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        estimate(
            root,
            "agent",
            &id,
            (1.0, 2.0, 3.0),
            (1.0, 2.0, 3.0),
            false,
            None,
            None,
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            &id,
            Action::Roadmap,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            &id,
            Action::Breakdown,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        task_new(root, "agent", &id, "Task", "", &[]).unwrap();
        transition(
            root,
            "agent",
            &id,
            Action::Start,
            None,
            Some(by),
            Some(statement),
            None,
        )
        .unwrap();
        id
    }

    #[test]
    fn cancel_cascades_to_nonterminal_tasks() {
        let dir = board();
        let root = dir.path();
        let id = to_doing(root);
        transition(
            root,
            "owner",
            &id,
            Action::Cancel,
            Some("superseded"),
            None,
            None,
            None,
        )
        .unwrap();
        let loaded = ProcessLoaded::load(root).unwrap();
        assert_eq!(loaded.find(&id).unwrap().state, State::Canceled);
        assert_eq!(loaded.find("#0001-01").unwrap().state, State::Canceled);
    }

    #[test]
    fn unblock_returns_to_the_recorded_state() {
        let dir = board();
        let root = dir.path();
        let id = to_doing(root);
        transition(
            root,
            "owner",
            &id,
            Action::Block,
            Some("waiting"),
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            ProcessLoaded::load(root).unwrap().find(&id).unwrap().state,
            State::Blocked
        );
        transition(root, "owner", &id, Action::Unblock, None, None, None, None).unwrap();
        assert_eq!(
            ProcessLoaded::load(root).unwrap().find(&id).unwrap().state,
            State::Doing
        );
    }
    #[test]
    fn request_changes_creates_a_rework_task() {
        let dir = board();
        let root = dir.path();
        let id = to_doing(root);
        transition(
            root,
            "agent",
            "#0001-01",
            Action::Work,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        transition(
            root,
            "agent",
            "#0001-01",
            Action::Done,
            None,
            None,
            None,
            Some("abc"),
        )
        .unwrap();
        transition(root, "agent", &id, Action::Handover, None, None, None, None).unwrap();
        review(
            root,
            "owner",
            &id,
            "member",
            "member",
            Verdict::ChangesRequired,
            &[("F1".to_owned(), "Fix it".to_owned())],
            1,
        )
        .unwrap();
        transition(
            root,
            "owner",
            &id,
            Action::RequestChanges,
            Some("fix it"),
            None,
            None,
            None,
        )
        .unwrap();
        let loaded = ProcessLoaded::load(root).unwrap();
        assert_eq!(loaded.find(&id).unwrap().state, State::Doing);
        let rework = loaded.find("#0001-02").unwrap();
        assert_eq!(rework.source_review.as_deref(), Some("RV-0001-01-01"));
        assert_eq!(rework.state, State::Pending);
    }
}
