use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use mdboard_core::Actors;
use mdboard_core::process::{
    Action, Kind, ProcessCard, State as ProcessState, TRANSITIONS, Verdict,
};
use mdboard_driver::process::ProcessLoaded;
use mdboard_serve::ServeOptions;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod migration;
mod mirror;
mod project_version;

#[derive(Debug, Parser)]
#[command(
    name = "mdboard",
    version,
    about = "A Markdown-native, Git-backed board engine",
    arg_required_else_help = true
)]
struct Cli {
    /// Repository root containing mdboard.toml.
    #[arg(long, global = true, default_value = ".")]
    root: PathBuf,

    /// Machine-readable output.
    #[arg(long, global = true)]
    json: bool,

    /// Actor id for write commands; roles come from actors.toml.
    #[arg(long, global = true)]
    actor: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum KindArg {
    Proposal,
    Defect,
    Research,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CommitClass {
    Human,
    Agent,
    Mdboard,
}

impl From<KindArg> for mdboard_core::process::Kind {
    fn from(value: KindArg) -> Self {
        match value {
            KindArg::Proposal => Self::Proposal,
            KindArg::Defect => Self::Defect,
            KindArg::Research => Self::Research,
        }
    }
}

#[derive(Debug, Subcommand)]
enum PersonCommand {
    Add {
        actor_id: String,
        #[arg(long)]
        display_name: String,
        #[arg(long, default_value = "member")]
        role: String,
        #[arg(long, default_value = "human")]
        kind: String,
        #[arg(long)]
        git_email: Option<String>,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        organisation: Option<String>,
        #[arg(long)]
        seats: Option<u32>,
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        guardian: bool,
        #[arg(long)]
        co_guardian: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Debug, Subcommand)]
enum SandboxCommand {
    Create {
        name: String,
    },
    Status,
    Leave,
    Delete {
        name: String,
        #[arg(long)]
        force: bool,
    },
    Reset {
        name: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
enum RotationCommand {
    /// List configured rotations.
    List,
    /// Show one rotation.
    Show { key: String },
    /// Rotate one responsibility.
    Next { key: String },
    /// Show the entire rotation history from events.
    History { key: String },
    /// List rotations that are currently due.
    Due,
    /// Perform one rotation for each due auto-rotation.
    Auto,
    /// Set the current holder manually.
    Set {
        key: String,
        #[arg(long)]
        person: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum VoteChoiceArg {
    Approve,
    Reject,
    Abstain,
}

impl From<VoteChoiceArg> for mdboard_core::process::VoteChoice {
    fn from(value: VoteChoiceArg) -> Self {
        match value {
            VoteChoiceArg::Approve => Self::Approve,
            VoteChoiceArg::Reject => Self::Reject,
            VoteChoiceArg::Abstain => Self::Abstain,
        }
    }
}

#[derive(Debug, Subcommand)]
enum VoteCommand {
    /// Open a roadmap vote for a pitched card; guardian only.
    Open {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Record one stakeholder's result.
    Record {
        id: String,
        #[arg(long)]
        email: String,
        #[arg(long, value_enum)]
        choice: VoteChoiceArg,
    },
    /// Veto an open vote; guardian or co-guardian.
    Veto {
        id: String,
        #[arg(long)]
        reason: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Resolve an open vote.
    Resolve {
        id: String,
        #[arg(long, value_enum)]
        tie_break: Option<VoteChoiceArg>,
    },
    /// Close an open vote as unresolved.
    Unresolved {
        id: String,
        #[arg(long)]
        reason: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Print the latest vote for a card.
    Show { id: String },
}

#[derive(Debug, Subcommand)]
enum StakeholderCommand {
    /// List stakeholders and seat expiry.
    List,
    /// Add a stakeholder seat.
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        organisation: Option<String>,
        #[arg(long, default_value_t = 1)]
        seats: u32,
        #[arg(long)]
        until: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Remove a stakeholder seat.
    Remove {
        #[arg(long)]
        email: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Renew a stakeholder seat.
    Renew {
        #[arg(long)]
        email: String,
        /// New expiry date (YYYY-MM-DD). Defaults to today plus seat_term_days.
        #[arg(long)]
        until: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Debug, Subcommand)]
enum GuardianCommand {
    /// Show the guardian.
    Show,
    /// Replace the guardian identity. Co-guardians are preserved.
    Transfer {
        #[arg(long)]
        name: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        organisation: Option<String>,
        #[arg(long)]
        ssh_key: Option<String>,
        #[arg(long)]
        ssh_fingerprint: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// List co-guardians.
    CoList,
    /// Add a co-guardian.
    CoAdd {
        #[arg(long)]
        name: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        organisation: Option<String>,
        #[arg(long)]
        ssh_key: Option<String>,
        #[arg(long)]
        ssh_fingerprint: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Remove a co-guardian by e-mail.
    CoRemove {
        #[arg(long)]
        email: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Add or replace a public SSH key for the guardian or a co-guardian.
    SshAdd {
        #[arg(long)]
        email: String,
        #[arg(long)]
        key: PathBuf,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Remove the SSH key for the guardian or a co-guardian.
    SshRemove {
        #[arg(long)]
        email: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Show configured SSH keys and fingerprints.
    SshShow,
    /// Create a fresh SSH challenge for a card.
    SshChallenge { card: String },
    /// Verify a signature over a card challenge.
    SshVerify {
        card: String,
        #[arg(long)]
        nonce: String,
        #[arg(long)]
        signature: PathBuf,
        #[arg(long)]
        email: String,
    },
}

#[derive(Debug, Subcommand)]
enum OrderCommand {
    /// List standing orders.
    List,
    /// Show a standing order and its history.
    Show { id: String },
    /// Propose a draft standing order.
    Propose {
        #[arg(long)]
        title: String,
        #[arg(long)]
        instruction: String,
        #[arg(long, default_value = "")]
        rationale: String,
        #[arg(long, default_value = "")]
        consequences: String,
        #[arg(long)]
        project: bool,
        #[arg(long = "kind")]
        kinds: Vec<String>,
        #[arg(long = "state")]
        states: Vec<String>,
        #[arg(long = "card")]
        cards: Vec<String>,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        until: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Activate a draft standing order.
    Add {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Amend an active standing order.
    Amend {
        id: String,
        #[arg(long)]
        instruction: String,
        #[arg(long)]
        reason: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Retire an active or draft standing order.
    Retire {
        id: String,
        #[arg(long)]
        reason: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Debug, Subcommand)]
enum AttachmentCommand {
    /// List a card's attachment files.
    List { id: String },
    /// Print the repository-relative path of one attachment.
    Path { id: String, name: String },
}

#[derive(Debug, Clone, Copy)]
struct CliRef<'a> {
    root: &'a Path,
    json: bool,
    actor: Option<&'a str>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a minimal board skeleton.
    Init {
        #[arg(default_value = ".")]
        dir: PathBuf,
        #[arg(long)]
        force: bool,
        /// Existing branch used as the upstream development branch.
        #[arg(long, visible_alias = "integration")]
        upstream: Option<String>,
        /// Subscription or payment model used for estimated EUR costs.
        #[arg(long)]
        subscription: Option<String>,
        /// Blended cost in EUR per one million tokens (input and output).
        #[arg(long)]
        eur_per_million_tokens: Option<f64>,
        /// Maximum number of committed roadmap batches.
        #[arg(long, default_value_t = 5)]
        roadmap_limit: u32,
        /// Initialize an empty Git repository when the directory is not one.
        #[arg(long)]
        init_git: bool,
        /// Agent commit name (default: agent).
        #[arg(long)]
        agent_git_name: Option<String>,
        /// Agent commit e-mail (default: no-replay@example.com).
        #[arg(long)]
        agent_git_email: Option<String>,
        /// Never prompt; use defaults for unset agent identity.
        #[arg(long)]
        no_prompt: bool,
        /// Public SSH key for the guardian; omit to leave SSH authentication disabled.
        #[arg(long)]
        ssh_key: Option<PathBuf>,
    },
    /// Check for mdBoard/project version updates.
    Update {
        /// Report only; do not change anything.
        #[arg(long)]
        check: bool,
    },
    /// Show the board summary.
    Status,
    /// Validate the board.
    Check {
        /// Also check Git upstream and owner-only files.
        #[arg(long)]
        git: bool,
    },
    /// Upgrade the board configuration to a newer schema.
    Migrate {
        /// Target schema version.
        #[arg(long, default_value_t = 3)]
        to: u32,
        /// Approve the printed migration summary and execute it.
        #[arg(long)]
        yes: bool,
    },
    /// Generate the Markdown views.
    Render {
        /// Compare instead of writing; exits 1 if a view differs.
        #[arg(long)]
        check: bool,
    },
    /// Rebuild the Markdown views and HTML site on file changes.
    Watch,
    /// Serve the HTML board locally and live-reload it.
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 3000)]
        port: u16,
        #[arg(long)]
        open: bool,
        #[arg(long)]
        no_watch: bool,
    },
    /// Print the machine-readable schema.
    Schema,
    /// Print a compact agent briefing.
    Context,
    /// Print the agent runbook.
    Runbook,
    /// Print the post-transition briefing for the current process board.
    Brief,
    /// List the legal named actions for a card.
    Actions { id: String },
    /// Inspect card-scoped attachments under .mdboard/attachments/<id>/.
    Attachment {
        #[command(subcommand)]
        command: AttachmentCommand,
    },
    /// Print a card's event history.
    History { id: String },
    /// Mark a task obsolete.
    Obsolete {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// Reopen an obsolete task.
    ReopenTask {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// Retire or delete an actor.
    ActorRemove {
        actor_id: String,
        #[arg(long)]
        retire: bool,
        #[arg(long)]
        delete: bool,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Add a person as actor and optional stakeholder/guardian.
    Person {
        #[command(subcommand)]
        command: PersonCommand,
    },
    /// Archive a card without deleting it.
    Archive {
        id: String,
        #[arg(long)]
        reason: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Restore an archived card.
    Unarchive {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Create an intent card.
    New {
        #[arg(long, value_enum)]
        kind: KindArg,
        title: String,
        #[arg(long, default_value = "")]
        body: String,
        #[arg(long, default_value = "owner")]
        intent_owner: String,
    },
    /// List card kinds.
    Kinds,
    /// Print a complete context-exhaustion handover prompt.
    HandoverPrompt { card: Option<String> },
    /// List archived cards.
    Archived,
    /// Sandbox branch operations.
    Sandbox {
        #[command(subcommand)]
        command: SandboxCommand,
    },
    /// Rotation operations.
    Rotation {
        #[command(subcommand)]
        command: RotationCommand,
    },
    /// Record the agent quality-gate result for a draft/refine card.
    QualityGate {
        card: String,
        #[arg(long)]
        passed: bool,
        #[arg(long = "finding")]
        findings: Vec<String>,
    },
    /// Return the next dependency-ready task in a batch.
    Next { batch: String },
    /// Summarise a batch and its task progress.
    BatchStatus { batch: String },
    /// Open an idea for refinement.
    Refine {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Pitch a refined idea to the consortium.
    Pitch {
        id: String,
        #[arg(long)]
        force_quality_gate: bool,
        #[arg(long)]
        reason: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Accept a pitched intent card and promote it to a batch.
    Accept {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Reject a pitched idea.
    RejectIdea {
        id: String,
        #[arg(long)]
        reason: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Park a refined idea back to spark.
    Park {
        id: String,
        #[arg(long)]
        reason: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Reopen a rejected idea as a spark.
    Reopen {
        id: String,
        #[arg(long)]
        reason: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Estimate a batch before roadmap.
    Estimate {
        id: String,
        #[command(flatten)]
        estimate: EstimateArgs,
    },
    /// Replace an estimate with a reason.
    Reestimate {
        id: String,
        #[command(flatten)]
        estimate: EstimateArgs,
        #[arg(long)]
        reason: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Commit an estimated batch to the roadmap.
    Roadmap {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Remove a batch from the roadmap.
    Unroadmap {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Move a roadmap batch to breakdown.
    Breakdown {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Abort breakdown and return to roadmap.
    AbortBreakdown {
        id: String,
        #[arg(long)]
        reason: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Start implementation after clarification.
    Start {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Move a doing batch to handover.
    Handover { id: String },
    /// Commit the current working tree and print the commit SHA.
    Commit {
        #[arg(short, long)]
        message: String,
        /// Commit identity class: human, agent or mdboard.
        #[arg(long, value_enum)]
        identity: Option<CommitClass>,
        /// Human actor for human identity; defaults to the executing actor.
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Move a pending task to working.
    Work { id: String },
    /// Complete a working task with its commit.
    Done {
        id: String,
        #[arg(long)]
        commit: String,
    },
    /// Move a working task to error.
    Fail {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// Recover an error task to pending.
    Recover {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Task operations.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    /// Stakeholder vote operations.
    Vote {
        #[command(subcommand)]
        command: VoteCommand,
    },
    /// Stakeholder seat operations.
    Stakeholder {
        #[command(subcommand)]
        command: StakeholderCommand,
    },
    /// Guardian and co-guardian operations.
    Guardian {
        #[command(subcommand)]
        command: GuardianCommand,
    },
    /// Standing-order operations.
    Order {
        #[command(subcommand)]
        command: OrderCommand,
    },
    /// Mirror a private checkout to a public target with exclusions.
    Mirror {
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        push: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        confirm_first_push: bool,
    },
    /// Record a human review artifact for a batch.
    Review {
        batch: String,
        #[arg(long)]
        reviewer: String,
        #[arg(long)]
        authored_by: String,
        #[arg(long, value_parser = parse_verdict)]
        verdict: Verdict,
        #[arg(long = "finding", value_name = "ID:TEXT")]
        findings: Vec<String>,
        #[arg(long, default_value_t = 1)]
        round: u32,
    },
    /// Request changes on a pitch or a handover batch.
    RequestChanges {
        id: String,
        #[arg(long)]
        reason: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Complete a reviewed batch.
    Complete {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Block a batch.
    Block {
        id: String,
        #[arg(long)]
        reason: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Return a blocked batch to its previous state.
    Unblock {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Cancel an idea or batch.
    Cancel {
        id: String,
        #[arg(long)]
        reason: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Upstream iteration operations.
    Upstream {
        #[command(subcommand)]
        command: UpstreamCommand,
    },
    /// Meeting gates.
    Meeting {
        #[command(subcommand)]
        command: MeetingCommand,
    },
    /// Print flow counts.
    Stats,
    /// Show the configured actors.
    Whoami,
    /// Inspect and verify owner-approved plugin manifests.
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
}

#[derive(Debug, Subcommand)]
enum PluginCommand {
    /// List configured plugins without executing them.
    List,
    /// Verify plugin commands against their pinned SHA-256.
    Verify {
        /// Plugin name; defaults to all configured plugins.
        name: Option<String>,
    },
    /// Write mdboard-plugins.lock from the owner-approved configuration.
    Lock,
}

const PLUGIN_LOCK: &str = "mdboard-plugins.lock";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PluginLock {
    schema: u32,
    #[serde(default)]
    plugins: Vec<PluginLockEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PluginLockEntry {
    name: String,
    kind: String,
    version: String,
    command: String,
    sha256: String,
    #[serde(default)]
    capabilities: Vec<String>,
    approved_by: String,
    approved_commit: String,
}

#[derive(Debug, Args)]
struct AuthArgs {
    /// Human actor who authorised a agent-executed gate.
    #[arg(long)]
    authorised_by: Option<String>,
    /// The human's authorisation statement.
    #[arg(long)]
    authorization: Option<String>,
}

#[derive(Debug, Args)]
struct EstimateArgs {
    #[arg(long)]
    tokens_optimistic: f64,
    #[arg(long)]
    tokens_realistic: f64,
    #[arg(long)]
    tokens_pessimistic: f64,
    #[arg(long)]
    agent_hours_optimistic: f64,
    #[arg(long)]
    agent_hours_realistic: f64,
    #[arg(long)]
    agent_hours_pessimistic: f64,
}

#[derive(Debug, Subcommand)]
enum TaskCommand {
    /// Create a task during breakdown.
    New {
        parent: String,
        title: String,
        #[arg(long, default_value = "")]
        body: String,
        #[arg(long = "depends-on")]
        depends_on: Vec<String>,
    },
    /// Move a pending task to working.
    Work { id: String },
    /// Complete a working task with its commit.
    Done {
        id: String,
        #[arg(long)]
        commit: String,
    },
    /// Move a working task to error.
    Fail {
        id: String,
        #[arg(long)]
        reason: String,
    },
    /// Recover an error task to pending.
    Recover {
        id: String,
        #[command(flatten)]
        auth: AuthArgs,
    },
}

#[derive(Debug, Subcommand)]
enum MeetingCommand {
    /// List pending, deferred and all meetings.
    List,
    /// Open an on-demand meeting gate.
    Open {
        #[arg(value_parser = parse_meeting_kind)]
        kind: mdboard_core::process::MeetingKind,
        #[arg(long)]
        batch: String,
        #[arg(long, default_value = "")]
        trigger: String,
    },
    /// Clarification questions.
    Question {
        #[command(subcommand)]
        command: QuestionCommand,
    },
    /// Resolve a clarification question.
    Resolve { meeting: String, question: String },
    /// Accept a meeting gate.
    Accept { meeting: String },
    /// Defer a meeting gate.
    Defer {
        meeting: String,
        #[arg(long)]
        until: String,
    },
    /// Dismiss a meeting gate with a reason.
    Dismiss {
        meeting: String,
        #[arg(long)]
        reason: String,
    },
}

#[derive(Debug, Subcommand)]
enum QuestionCommand {
    /// Add a clarification question.
    Add { meeting: String, text: String },
    /// Resolve a clarification question.
    Resolve { meeting: String, question: String },
}

#[derive(Debug, Subcommand)]
enum UpstreamCommand {
    /// Open the one active upstream iteration.
    Start {
        /// Base revision; defaults to HEAD.
        #[arg(long, default_value = "HEAD")]
        base: String,
    },
    /// Show the current upstream iteration.
    Status,
    /// Renew the writer lease.
    Renew {
        #[arg(long, default_value_t = 12)]
        hours: i64,
    },
    /// Clear the writer lease; owner only.
    Clear,
    /// Merge the open iteration into nightly; owner only.
    Merge,
    /// Release nightly into stable; owner only.
    Release,
}

fn run_plugin_command(ctx: &CliRef, command: PluginCommand) -> Result<()> {
    let loaded = ProcessLoaded::load(ctx.root)?;
    let lock = load_plugin_lock(ctx.root)?;
    let mut failed = false;
    match command {
        PluginCommand::List => {
            if loaded.plugins.is_empty() {
                println!("no plugins configured");
            } else if ctx.json {
                let entries = loaded
                    .plugins
                    .iter()
                    .map(|(name, plugin)| {
                        serde_json::json!({
                            "name": name,
                            "kind": plugin.kind,
                            "version": plugin.version,
                            "enabled": plugin.enabled,
                            "network": plugin.network,
                            "locked": lock.as_ref().is_some_and(|lock| lock.plugins.iter().any(|entry| entry.name == *name)),
                            "digest": plugin_digest(&plugin_path(ctx.root, plugin), &plugin.sha256),
                        })
                    })
                    .collect::<Vec<_>>();
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else {
                for (name, plugin) in &loaded.plugins {
                    println!(
                        "{name}: {} {} (enabled={}, network={}, locked={}, digest={})",
                        plugin.kind,
                        plugin.version,
                        plugin.enabled,
                        plugin.network,
                        lock.as_ref().is_some_and(|lock| lock
                            .plugins
                            .iter()
                            .any(|entry| entry.name == *name)),
                        plugin_digest(&plugin_path(ctx.root, plugin), &plugin.sha256)
                    );
                }
            }
        }
        PluginCommand::Verify { name } => {
            for (plugin_name, plugin) in &loaded.plugins {
                if name.as_deref().is_some_and(|filter| filter != plugin_name) {
                    continue;
                }
                let digest = plugin_digest(&plugin_path(ctx.root, plugin), &plugin.sha256);
                let locked = lock.as_ref().is_some_and(|lock| {
                    lock.plugins.iter().any(|entry| {
                        entry.name == *plugin_name
                            && entry.sha256.eq_ignore_ascii_case(&plugin.sha256)
                    })
                });
                if digest != "ok" || !locked {
                    failed = true;
                    println!("{plugin_name}: digest={digest} locked={locked}");
                } else {
                    println!("{plugin_name}: ok");
                }
            }
        }
        PluginCommand::Lock => {
            let actor = require_actor(ctx.actor)?;
            let actors = Actors::load_optional(&loaded.process_root().join("actors.toml"))?;
            let owner = actors
                .get(&actor)
                .is_some_and(|entry| entry.has_role("owner"));
            if !owner {
                bail!("only the owner may write the plugin lockfile");
            }
            let commit = mdboard_git::state(ctx.root)
                .map(|state| state.commit)
                .unwrap_or_else(|_| "unversioned".to_owned());
            let entries = loaded
                .plugins
                .iter()
                .map(|(name, plugin)| PluginLockEntry {
                    name: name.clone(),
                    kind: plugin.kind.clone(),
                    version: plugin.version.clone(),
                    command: plugin.command.display().to_string(),
                    sha256: plugin.sha256.clone(),
                    capabilities: plugin.capabilities.clone(),
                    approved_by: actor.clone(),
                    approved_commit: commit.clone(),
                })
                .collect::<Vec<_>>();
            let lock = PluginLock {
                schema: 1,
                plugins: entries,
            };
            let path = ctx.root.join(PLUGIN_LOCK);
            std::fs::write(&path, toml::to_string_pretty(&lock)?)?;
            println!(
                "wrote {} ({} plugins, approved by {actor})",
                path.display(),
                lock.plugins.len()
            );
        }
    }
    if failed {
        bail!("plugin verification failed");
    }
    Ok(())
}

fn load_plugin_lock(root: &Path) -> Result<Option<PluginLock>> {
    let path = root.join(PLUGIN_LOCK);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let lock: PluginLock =
        toml::from_str(&text).with_context(|| format!("cannot parse {}", path.display()))?;
    if lock.schema != 1 {
        bail!("unsupported plugin lock schema {}", lock.schema);
    }
    Ok(Some(lock))
}

fn plugin_path(root: &Path, plugin: &mdboard_core::config::PluginConfig) -> PathBuf {
    if plugin.command.is_absolute() {
        plugin.command.clone()
    } else {
        root.join(&plugin.command)
    }
}

fn plugin_digest(path: &Path, expected: &str) -> &'static str {
    let Ok(bytes) = std::fs::read(path) else {
        return "missing";
    };
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual.eq_ignore_ascii_case(expected) {
        "ok"
    } else {
        "mismatch"
    }
}

#[derive(Debug, serde::Serialize)]
struct ProcessFinding {
    level: &'static str,
    code: &'static str,
    message: String,
}

fn process_check(loaded: &ProcessLoaded, git: bool) -> Vec<ProcessFinding> {
    let mut findings = Vec::new();
    let active = loaded
        .cards
        .iter()
        .filter(|card| {
            card.kind == mdboard_core::process::Kind::Batch
                && matches!(card.state, ProcessState::Breakdown | ProcessState::Doing)
        })
        .collect::<Vec<_>>();
    if active.len() > 1 {
        findings.push(ProcessFinding {
            level: "error",
            code: "PROC-001",
            message: format!(
                "{} batches are active ({}); only one may be in breakdown/doing",
                active.len(),
                active
                    .iter()
                    .map(|card| card.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }
    let working = loaded
        .cards
        .iter()
        .filter(|card| {
            card.kind == mdboard_core::process::Kind::Task && card.state == ProcessState::Working
        })
        .collect::<Vec<_>>();
    if working.len() > 1 {
        findings.push(ProcessFinding {
            level: "error",
            code: "PROC-002",
            message: format!(
                "{} tasks are working ({}); only one may be working",
                working.len(),
                working
                    .iter()
                    .map(|card| card.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }
    if loaded.roadmap_count() > loaded.roadmap_limit() {
        findings.push(ProcessFinding {
            level: "error",
            code: "PROC-003",
            message: format!(
                "roadmap has {} batches but the limit is {}",
                loaded.roadmap_count(),
                loaded.roadmap_limit()
            ),
        });
    }
    if git
        && let Some(iteration) = loaded
            .upstream
            .as_ref()
            .filter(|iteration| iteration.status == mdboard_core::process::IterationStatus::Open)
    {
        let expired = chrono::DateTime::parse_from_rfc3339(&iteration.lease_expires)
            .map_or(true, |expires| expires <= chrono::Utc::now());
        if expired {
            findings.push(ProcessFinding {
                level: "error",
                code: "UPSTREAM-001",
                message: format!(
                    "upstream iteration {} writer lease expired at {}",
                    iteration.iteration, iteration.lease_expires
                ),
            });
        }
        if let Ok(branch) = mdboard_git::current_branch(&loaded.root)
            && branch != loaded.policy.branches.upstream
        {
            findings.push(ProcessFinding {
                level: "error",
                code: "UPSTREAM-002",
                message: format!(
                    "current branch `{branch}` is not the upstream branch `{}`",
                    loaded.policy.branches.upstream
                ),
            });
        }
        if let Ok(dirty) = mdboard_git::is_dirty(&loaded.root)
            && dirty
        {
            findings.push(ProcessFinding {
                level: "warning",
                code: "UPSTREAM-003",
                message: "the working tree has uncommitted changes".to_owned(),
            });
        }
        if let Ok(paths) = mdboard_git::dirty_paths(&loaded.root) {
            let owner_only = [
                "LICENSE".to_owned(),
                "NOTICE".to_owned(),
                "mdboard.toml".to_owned(),
                format!("{}/actors.toml", loaded.paths.root),
                format!("{}/roadmap-override.md", loaded.paths.root),
            ];
            for path in paths {
                if owner_only.iter().any(|owned| owned == &path) {
                    findings.push(ProcessFinding {
                        level: "error",
                        code: "OWNER-001",
                        message: format!("owner-only file changed: `{path}`"),
                    });
                }
            }
        }
    }
    for card in &loaded.cards {
        if card.state == ProcessState::Completed {
            let Some(report) = &card.report else {
                findings.push(ProcessFinding {
                    level: "error",
                    code: "PROC-004",
                    message: format!("completed batch {} has no report", card.id),
                });
                continue;
            };
            if !loaded.root.join(report).exists() {
                findings.push(ProcessFinding {
                    level: "error",
                    code: "PROC-005",
                    message: format!("completed batch {} report is missing: {report}", card.id),
                });
            }
            if loaded
                .accepted_review_for(&card.id, &card.intent_owner)
                .is_none()
            {
                findings.push(ProcessFinding {
                    level: "error",
                    code: "PROC-006",
                    message: format!(
                        "completed batch {} has no independent accepted review",
                        card.id
                    ),
                });
            }
        }
    }
    findings.extend(layout_findings(loaded));
    findings.extend(attachment_findings(loaded));
    findings
}

fn layout_findings(loaded: &ProcessLoaded) -> Vec<ProcessFinding> {
    const DIRS: &[&str] = &[
        "cards",
        "events",
        "meetings",
        "reviews",
        "reports",
        "attachments",
        "votes",
        "orders",
        "site",
    ];
    const FILES: &[&str] = &[
        "upstream.md",
        "roadmap-override.md",
        "actors.toml",
        "mirror.toml",
    ];
    let root = loaded.process_root();
    let mut findings = Vec::new();
    if !root.is_dir() {
        findings.push(ProcessFinding {
            level: "error",
            code: "LAYOUT-002",
            message: format!("board root is missing: {}", root.display()),
        });
        return findings;
    }
    if !loaded.cards_dir().is_dir() {
        findings.push(ProcessFinding {
            level: "error",
            code: "LAYOUT-002",
            message: format!(
                "required directory is missing: {}",
                loaded.cards_dir().display()
            ),
        });
    }
    if !root.join("actors.toml").is_file() {
        findings.push(ProcessFinding {
            level: "error",
            code: "LAYOUT-002",
            message: format!(
                "required file is missing: {}",
                root.join("actors.toml").display()
            ),
        });
    }
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name == ".DS_Store" {
                continue;
            }
            let allowed_dir = DIRS.contains(&name);
            let allowed_file = FILES.contains(&name);
            if !allowed_dir && !allowed_file {
                findings.push(ProcessFinding {
                    level: "error",
                    code: "LAYOUT-001",
                    message: format!(
                        "unexpected top-level board entry: {}",
                        relative_display(&loaded.root, &path)
                    ),
                });
            } else if allowed_dir && !path.is_dir() {
                findings.push(ProcessFinding {
                    level: "error",
                    code: "LAYOUT-001",
                    message: format!(
                        "board entry must be a directory: {}",
                        relative_display(&loaded.root, &path)
                    ),
                });
            } else if allowed_file && !path.is_file() {
                findings.push(ProcessFinding {
                    level: "error",
                    code: "LAYOUT-001",
                    message: format!(
                        "board entry must be a file: {}",
                        relative_display(&loaded.root, &path)
                    ),
                });
            }
        }
    }
    findings
}

fn attachment_findings(loaded: &ProcessLoaded) -> Vec<ProcessFinding> {
    let mut findings = Vec::new();
    let prefix = loaded.attachment_prefix();
    for card in &loaded.cards {
        let slug = card.id.trim_start_matches('#');
        let own_prefix = format!("{prefix}{slug}/");
        for reference in attachment_references(&card.body, &prefix) {
            if !reference.starts_with(&own_prefix) {
                findings.push(ProcessFinding {
                    level: "error",
                    code: "ATTACH-001",
                    message: format!(
                        "{} references an attachment outside its own directory: {reference}",
                        card.id
                    ),
                });
                continue;
            }
            let path = Path::new(&reference);
            if path
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                findings.push(ProcessFinding {
                    level: "error",
                    code: "ATTACH-002",
                    message: format!("{} has an unsafe attachment path: {reference}", card.id),
                });
                continue;
            }
            if !loaded.root.join(path).is_file() {
                findings.push(ProcessFinding {
                    level: "error",
                    code: "ATTACH-003",
                    message: format!("{} attachment is missing: {reference}", card.id),
                });
            }
        }
    }
    let attachments_root = loaded.attachments_root();
    if let Ok(entries) = std::fs::read_dir(&attachments_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if path.is_dir() {
                if loaded.find(&format!("#{name}")).is_none() {
                    findings.push(ProcessFinding {
                        level: "error",
                        code: "ATTACH-004",
                        message: format!(
                            "attachment directory has no card: {}",
                            relative_display(&loaded.root, &path)
                        ),
                    });
                }
            } else {
                findings.push(ProcessFinding {
                    level: "error",
                    code: "ATTACH-005",
                    message: format!(
                        "unexpected file directly under attachments root: {}",
                        relative_display(&loaded.root, &path)
                    ),
                });
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(loaded.cards_dir()) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                findings.push(ProcessFinding {
                    level: "error",
                    code: "ATTACH-006",
                    message: format!(
                        "card directories are not supported; attachments belong under {}: {}",
                        relative_display(&loaded.root, &attachments_root),
                        relative_display(&loaded.root, &entry.path())
                    ),
                });
            }
        }
    }
    findings
}

fn attachment_references(body: &str, prefix: &str) -> Vec<String> {
    let mut references = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find(prefix) {
        let tail = &rest[start..];
        let end = tail
            .char_indices()
            .find(|(_, ch)| {
                ch.is_whitespace()
                    || matches!(ch, ')' | ']' | '>' | '<' | '"' | '\'' | '`' | ',' | ';')
            })
            .map_or(tail.len(), |(index, _)| index);
        let reference = tail[..end]
            .trim_end_matches(['.', ':', ',', ';'])
            .to_owned();
        if !reference.is_empty()
            && !reference.ends_with('/')
            && !reference.contains('<')
            && !reference.contains('>')
        {
            references.push(reference);
        }
        rest = &tail[end..];
    }
    references
}

fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn migrate_config(root: &Path, to: u32, yes: bool) -> Result<()> {
    if to != 3 {
        bail!("only schema 3 is supported");
    }
    let path = root.join("mdboard.toml");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let mut value: toml::Value =
        toml::from_str(&text).with_context(|| format!("cannot parse {}", path.display()))?;
    let table = value
        .as_table_mut()
        .context("mdboard.toml must be a table")?;
    let schema = table
        .get("schema")
        .and_then(toml::Value::as_integer)
        .unwrap_or(0);
    if schema == 3 {
        println!("mdboard migrate: already schema 3");
        return Ok(());
    }
    if schema != 2 {
        bail!("cannot migrate schema {schema} to 3");
    }
    let summary = migration::schema_summary(
        2,
        3,
        generated_version::VERSION,
        generated_version::BUILD_DATE,
    );
    print!("{}", summary.render());
    if !yes {
        bail!("migration not executed; re-run with --yes to approve the summary");
    }
    table.insert("schema".to_owned(), toml::Value::Integer(3));
    if !table.contains_key("stakeholders") {
        let mut stakeholders = toml::map::Map::new();
        stakeholders.insert("max_seats".to_owned(), toml::Value::Integer(5));
        stakeholders.insert("quorum_seats".to_owned(), toml::Value::Integer(3));
        stakeholders.insert("seat_term_days".to_owned(), toml::Value::Integer(365));
        stakeholders.insert("seat_warning_days".to_owned(), toml::Value::Integer(90));
        stakeholders.insert("members".to_owned(), toml::Value::Array(Vec::new()));
        table.insert("stakeholders".to_owned(), toml::Value::Table(stakeholders));
    }
    if !table.contains_key("guardian") {
        let mut guardian = toml::map::Map::new();
        guardian.insert("name".to_owned(), toml::Value::String("Owner".to_owned()));
        guardian.insert(
            "email".to_owned(),
            toml::Value::String("owner@example.com".to_owned()),
        );
        guardian.insert(
            "organisation".to_owned(),
            toml::Value::String(String::new()),
        );
        table.insert("guardian".to_owned(), toml::Value::Table(guardian));
    }
    let rendered = toml::to_string_pretty(&value)?;
    std::fs::write(&path, rendered).with_context(|| format!("cannot write {}", path.display()))?;
    let loaded = ProcessLoaded::load(root)?;
    println!(
        "mdboard migrate: schema {} -> 3; stakeholders {} seats, guardian {}",
        schema, loaded.policy.stakeholders.max_seats, loaded.policy.guardian.email
    );
    Ok(())
}

fn governance_authorization(actor: Option<&str>, auth: &AuthArgs) -> Result<()> {
    if actor == Some("owner") {
        return Ok(());
    }
    if auth.authorised_by.as_deref() != Some("owner")
        || auth
            .authorization
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        bail!("governance changes require owner authorisation");
    }
    Ok(())
}

fn set_optional(table: &mut toml::map::Map<String, toml::Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        table.insert(key.to_owned(), toml::Value::String(value.to_owned()));
    } else {
        table.remove(key);
    }
}

fn edit_config(
    root: &Path,
    edit: impl FnOnce(&mut toml::map::Map<String, toml::Value>) -> Result<()>,
) -> Result<ProcessLoaded> {
    let path = root.join("mdboard.toml");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let mut value: toml::Value =
        toml::from_str(&text).with_context(|| format!("cannot parse {}", path.display()))?;
    let table = value
        .as_table_mut()
        .context("mdboard.toml must be a table")?;
    edit(table)?;
    std::fs::write(&path, toml::to_string_pretty(&value)?)
        .with_context(|| format!("cannot write {}", path.display()))?;
    ProcessLoaded::load(root)
}

fn build_order_scope(
    project: bool,
    kinds: &[String],
    states: &[String],
    cards: &[String],
    from: Option<String>,
    until: Option<String>,
) -> Result<mdboard_core::process::OrderScope> {
    use std::str::FromStr;
    let kinds = kinds
        .iter()
        .map(|value| mdboard_core::process::Kind::from_str(value))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let states = states
        .iter()
        .map(|value| mdboard_core::process::State::from_str(value))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(mdboard_core::process::OrderScope {
        project,
        kinds,
        states,
        cards: cards.to_vec(),
        from,
        until,
    })
}

fn print_order(order: &mdboard_core::process::Order, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(order)?);
    } else {
        println!("{} — {:?} — {}", order.id, order.status, order.title);
        println!("scope: {:?}", order.scope);
        println!("authorised by: {}", order.authorised_by);
        println!();
        println!("{}", order.instruction);
        if !order.rationale.is_empty() {
            println!("\nRationale: {}", order.rationale);
        }
        if !order.consequences.is_empty() {
            println!("\nConsequences: {}", order.consequences);
        }
        if !order.history.is_empty() {
            println!("\nHistory:");
            for version in &order.history {
                println!(
                    "- {} by {}: {}",
                    version.at,
                    version.by,
                    version.reason.as_deref().unwrap_or("")
                );
            }
        }
    }
    Ok(())
}

fn stakeholder_add(
    root: &Path,
    name: &str,
    email: &str,
    organisation: Option<&str>,
    seats: u32,
    until: &str,
) -> Result<()> {
    use toml_edit::{DocumentMut, Item, Table, value};
    let path = root.join("mdboard.toml");
    let original = std::fs::read_to_string(&path)?;
    let mut doc: DocumentMut = original.parse()?;
    let stakeholders = doc["stakeholders"]
        .as_table_mut()
        .context("stakeholders table")?;
    stakeholders
        .entry("members")
        .or_insert(Item::ArrayOfTables(toml_edit::ArrayOfTables::new()));
    let members = stakeholders["members"]
        .as_array_of_tables_mut()
        .context("members")?;
    if members.iter().any(|m| {
        m.get("email")
            .and_then(|v| v.as_str())
            .is_some_and(|v| v.eq_ignore_ascii_case(email))
    }) {
        bail!("stakeholder `{email}` already exists");
    }
    let mut member = Table::new();
    member.insert("name", value(name));
    member.insert("email", value(email));
    member.insert("organisation", value(organisation.unwrap_or("")));
    member.insert("seats", value(i64::from(seats)));
    member.insert("seat_until", value(until));
    members.push(member);
    std::fs::write(&path, doc.to_string())?;
    if let Err(error) = ProcessLoaded::load(root) {
        std::fs::write(&path, original)?;
        return Err(error);
    }
    Ok(())
}

fn stakeholder_remove(root: &Path, email: &str) -> Result<()> {
    use toml_edit::DocumentMut;
    let loaded = ProcessLoaded::load(root)?;
    if loaded
        .votes
        .iter()
        .any(|vote| format!("{:?}", vote.status) == "Open")
    {
        bail!("cannot remove a stakeholder while a vote is open");
    }
    let path = root.join("mdboard.toml");
    let original = std::fs::read_to_string(&path)?;
    let mut doc: DocumentMut = original.parse()?;
    let members = doc["stakeholders"]["members"]
        .as_array_of_tables_mut()
        .context("members")?;
    let before = members.len();
    members.retain(|m| {
        m.get("email")
            .and_then(|v| v.as_str())
            .is_none_or(|v| !v.eq_ignore_ascii_case(email))
    });
    if members.len() == before {
        bail!("stakeholder `{email}` not found");
    }
    std::fs::write(&path, doc.to_string())?;
    if let Err(error) = ProcessLoaded::load(root) {
        std::fs::write(&path, original)?;
        return Err(error);
    }
    Ok(())
}

fn actor_remove(root: &Path, actor_id: &str, retire: bool, delete: bool) -> Result<()> {
    use toml_edit::DocumentMut;
    if retire == delete {
        bail!("choose exactly one of --retire or --delete");
    }
    let path = root.join(".mdboard/actors.toml");
    let original = std::fs::read_to_string(&path)?;
    let mut doc: DocumentMut = original.parse()?;
    let actors = doc["actors"].as_table_mut().context("actors table")?;
    if !actors.contains_key(actor_id) {
        bail!("actor `{actor_id}` does not exist");
    }
    if delete {
        let loaded = ProcessLoaded::load(root)?;
        let needles = [
            serde_json::to_string(&loaded.cards)?,
            serde_json::to_string(&loaded.events)?,
            serde_json::to_string(&loaded.reviews)?,
        ];
        if needles.iter().any(|text| text.contains(actor_id)) {
            bail!("actor `{actor_id}` has historical references; use --retire");
        }
        actors.remove(actor_id);
    } else {
        actors[actor_id]["retired"] = toml_edit::value(true);
    }
    std::fs::write(&path, doc.to_string())?;
    if let Err(error) = ProcessLoaded::load(root) {
        std::fs::write(&path, original)?;
        return Err(error);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn person_add(
    root: &Path,
    actor_id: &str,
    display_name: &str,
    role: &str,
    kind: &str,
    git_email: Option<&str>,
    email: Option<&str>,
    organisation: Option<&str>,
    seats: Option<u32>,
    until: Option<&str>,
    guardian: bool,
    co_guardian: bool,
) -> Result<()> {
    use toml_edit::{Array, DocumentMut, Item, Table, Value, value};
    if actor_id.trim().is_empty() || display_name.trim().is_empty() {
        bail!("actor id and display name are required");
    }
    let actors_path = root.join(".mdboard/actors.toml");
    let config_path = root.join("mdboard.toml");
    let actors_original = std::fs::read_to_string(&actors_path)?;
    let config_original = std::fs::read_to_string(&config_path)?;
    let mut actors: DocumentMut = actors_original.parse()?;
    let mut config: DocumentMut = config_original.parse()?;
    actors["actors"].or_insert(Item::Table(Table::new()));
    let actors_table = actors["actors"].as_table_mut().context("actors table")?;
    if actors_table.contains_key(actor_id) {
        bail!("actor `{actor_id}` already exists");
    }
    let mut actor = Table::new();
    actor.insert("kind", value(kind));
    actor.insert("display_name", value(display_name));
    let mut roles = Array::new();
    roles.push(role);
    actor.insert("roles", Item::Value(Value::Array(roles)));
    if let Some(email) = git_email {
        let mut emails = Array::new();
        emails.push(email);
        actor.insert("git_emails", Item::Value(Value::Array(emails)));
    }
    let handles = Table::new();
    actor.insert("handles", Item::Table(handles));
    actors_table.insert(actor_id, Item::Table(actor));

    if let Some(email) = email.filter(|_| seats.is_some() || until.is_some()) {
        let seats = seats.unwrap_or(1);
        let until = until.context("--until is required when creating a stakeholder seat")?;
        config["stakeholders"]
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .context("stakeholders table")?
            .entry("members")
            .or_insert(Item::ArrayOfTables(toml_edit::ArrayOfTables::new()));
        let members = config["stakeholders"]["members"]
            .as_array_of_tables_mut()
            .context("stakeholders.members")?;
        let mut member = Table::new();
        member.insert("name", value(display_name));
        member.insert("email", value(email));
        member.insert("organisation", value(organisation.unwrap_or("")));
        member.insert("seats", value(i64::from(seats)));
        member.insert("seat_until", value(until));
        members.push(member);
    }
    if guardian || co_guardian {
        let email = email.context("--email is required for guardian changes")?;
        config["guardian"]
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .context("guardian table")?;
        let g = config["guardian"]
            .as_table_mut()
            .context("guardian table")?;
        if guardian {
            g.insert("name", value(display_name));
            g.insert("email", value(email));
            g.insert("organisation", value(organisation.unwrap_or("")));
        }
        if co_guardian {
            let approved_by = g
                .get("email")
                .and_then(|value| value.as_str())
                .unwrap_or("guardian")
                .to_owned();
            g.entry("co_guardians")
                .or_insert(Item::ArrayOfTables(toml_edit::ArrayOfTables::new()));
            let co = g["co_guardians"]
                .as_array_of_tables_mut()
                .context("guardian.co_guardians")?;
            let mut item = Table::new();
            item.insert("name", value(display_name));
            item.insert("email", value(email));
            item.insert("organisation", value(organisation.unwrap_or("")));
            item.insert("approved_by", value(approved_by));
            co.push(item);
        }
    }
    std::fs::write(&actors_path, actors.to_string())?;
    std::fs::write(&config_path, config.to_string())?;
    if let Err(error) = ProcessLoaded::load(root) {
        std::fs::write(&actors_path, actors_original)?;
        std::fs::write(&config_path, config_original)?;
        return Err(error);
    }
    Ok(())
}

fn print_vote(vote: &mdboard_core::process::Vote, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(vote)?);
    } else {
        println!("{} — {} — {:?}", vote.id, vote.card, vote.status);
        println!(
            "  allocated: {} seats · counted: {}",
            vote.allocated_seats, vote.counted_seats
        );
        for item in &vote.votes {
            println!(
                "  {} <{}> {} seats: {:?}",
                item.name, item.email, item.seats, item.choice
            );
        }
    }
    Ok(())
}

fn process_views(loaded: &ProcessLoaded) -> Vec<(PathBuf, String)> {
    let visible: Vec<_> = loaded
        .cards
        .iter()
        .filter(|card| card.archived_at.is_none())
        .cloned()
        .collect();
    let mut views: Vec<_> = mdboard_renderer::process::render_process_views(
        &visible,
        &loaded.policy,
        &loaded.meetings,
        loaded.upstream.as_ref(),
        &loaded.root,
        loaded.roadmap_limit(),
    )
    .files
    .into_iter()
    .collect();
    if let Some(rotations) = &loaded.rotations {
        let mut text = String::from("# Rotations\n\n");
        for (key, rotation) in &rotations.rotations {
            text.push_str(&format!(
                "## {} — {}\n\n{}\n\n- strategy: {:?}\n- current: {}\n",
                key, rotation.title, rotation.description, rotation.strategy, rotation.current
            ));
            if let Some(days) = rotation.auto_rotate_days {
                text.push_str(&format!("- rotate every {days} days\n"));
                if let Some(last) = &rotation.last_rotated
                    && let Ok(last) = chrono::DateTime::parse_from_rfc3339(last)
                {
                    let due = last + chrono::Duration::days(i64::from(days));
                    text.push_str(&format!("- next due: {}\n", due.to_rfc3339()));
                }
            }
            text.push('\n');
        }
        views.push((loaded.root.join("ROTATIONS.md"), text));
    }
    views
}

fn refresh_views(ctx: &CliRef) -> Result<()> {
    if let Ok(loaded) = ProcessLoaded::load(ctx.root) {
        for (path, content) in process_views(&loaded) {
            std::fs::write(path, content)?;
        }
    }
    Ok(())
}

fn require_actor(actor: Option<&str>) -> Result<String> {
    actor
        .map(str::to_owned)
        .context("--actor is required for this command")
}

fn run_transition(
    ctx: &CliRef,
    id: &str,
    action: Action,
    reason: Option<&str>,
    authorised_by: Option<&str>,
    authorization: Option<&str>,
    commit: Option<&str>,
) -> Result<()> {
    if let Ok(branch) = mdboard_git::current_branch(ctx.root)
        && branch.starts_with("sandbox/")
        && matches!(
            action,
            Action::AcceptIdea
                | Action::Handover
                | Action::Complete
                | Action::Merge
                | Action::Release
        )
    {
        bail!("this gate is refused in a sandbox; return to upstream first");
    }
    let actor = require_actor(ctx.actor)?;
    let card = mdboard_actions::process::transition(
        ctx.root,
        &actor,
        id,
        action,
        reason,
        authorised_by,
        authorization,
        commit,
    )?;
    print_process_card(&card, ctx.json)?;
    refresh_views(ctx)
}

fn print_process_card(card: &ProcessCard, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(card)?);
        return Ok(());
    }
    println!("{} — {}", card.id, card.title);
    println!("  kind:   {}", card.kind);
    println!("  state:  {}", card.state);
    println!("  owner:  {}", card.intent_owner);
    if let Some(parent) = &card.parent {
        println!("  parent: {parent}");
    }
    if let Some(estimate) = &card.estimate {
        println!(
            "  estimate: {:.0} tokens, {:.2} agent hours (weighted)",
            estimate.tokens.weighted, estimate.agent_hours.weighted
        );
    }
    if let Some(report) = &card.report {
        println!("  report: {report}");
    }
    Ok(())
}

fn print_process_context(loaded: &ProcessLoaded, json: bool) -> Result<()> {
    let active = loaded.active_batches().next();
    let working = loaded.working_tasks().next();
    let pending = loaded.pending_meetings();
    let blocked = loaded
        .cards
        .iter()
        .filter(|card| card.state == ProcessState::Blocked && card.archived_at.is_none())
        .collect::<Vec<_>>();
    let errors = loaded
        .cards
        .iter()
        .filter(|card| card.state == ProcessState::Error && card.archived_at.is_none())
        .collect::<Vec<_>>();

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema": loaded.schema,
                "version": loaded.version,
                "branches": {
                    "stable": loaded.policy.branches.stable,
                    "nightly": loaded.policy.branches.nightly,
                    "upstream": loaded.policy.branches.upstream,
                },
                "iteration": loaded.upstream.as_ref().map(|iteration| serde_json::json!({
                    "iteration": iteration.iteration,
                    "status": iteration.status,
                    "base": iteration.base,
                    "writer": iteration.writer,
                })),
                "roadmap": {
                    "count": loaded.roadmap_count(),
                    "limit": loaded.roadmap_limit(),
                    "estimated_cost_eur": loaded.estimated_cost_eur(),
                },
                "standing_orders": {
                    "active": loaded.active_orders().len(),
                    "draft": loaded.orders.iter().filter(|order| order.status == mdboard_core::process::OrderStatus::Draft).count(),
                    "retired": loaded.orders.iter().filter(|order| order.status == mdboard_core::process::OrderStatus::Retired).count(),
                },
                "governance": {
                    "guardian": {
                        "name": loaded.policy.guardian.name,
                        "email": loaded.policy.guardian.email,
                    },
                    "ssh_enabled": mdboard_actions::process::has_ssh_keys(loaded),
                    "co_guardians": loaded.policy.guardian.co_guardians.len(),
                    "allocated_seats": loaded.policy.stakeholders.total_seats(),
                    "max_seats": loaded.policy.stakeholders.max_seats,
                    "quorum_seats": loaded.policy.stakeholders.quorum_seats,
                    "stakeholders": loaded.policy.stakeholders.members.len(),
                },
                "active_batch": active.map(|card| serde_json::json!({"id": card.id, "state": card.state})),
                "working_task": working.map(|card| serde_json::json!({"id": card.id, "state": card.state})),
                "pending_meetings": pending.iter().map(|meeting| serde_json::json!({
                    "id": meeting.id, "kind": meeting.kind, "status": meeting.status
                })).collect::<Vec<_>>(),
                "blocked": blocked.iter().map(|card| card.id.clone()).collect::<Vec<_>>(),
                "error_tasks": errors.iter().map(|card| card.id.clone()).collect::<Vec<_>>(),
                "cards": loaded.cards.len(),
            }))?
        );
        return Ok(());
    }

    println!("# mdboard context (1.0)\n");
    if let Ok(branch) = mdboard_git::current_branch(&loaded.root)
        && branch.starts_with("sandbox/")
    {
        println!("- MODE: SANDBOX — playground only (`{branch}`)");
        println!("  return: mdboard sandbox leave");
    }
    println!("- schema: {}", loaded.schema);
    println!(
        "- branches: `{}` (stable) ← `{}` (nightly) ← `{}` (upstream)",
        loaded.policy.branches.stable,
        loaded.policy.branches.nightly,
        loaded.policy.branches.upstream
    );
    match &loaded.upstream {
        Some(iteration) => println!(
            "- iteration: {} ({:?}), writer `{}`",
            iteration.iteration, iteration.status, iteration.writer
        ),
        None => println!("- iteration: none open"),
    }
    println!(
        "- roadmap: {} of {} batches, estimated €{:.4}",
        loaded.roadmap_count(),
        loaded.roadmap_limit(),
        loaded.estimated_cost_eur()
    );
    println!(
        "- governance: guardian `{}`, {}/{} seats, quorum {}",
        loaded.policy.guardian.name,
        loaded.policy.stakeholders.total_seats(),
        loaded.policy.stakeholders.max_seats,
        loaded.policy.stakeholders.quorum_seats
    );
    println!("- standing orders: {} active", loaded.active_orders().len());
    if let Some(rotations) = &loaded.rotations {
        for (key, rotation) in &rotations.rotations {
            let Some(days) = rotation.auto_rotate_days else {
                continue;
            };
            let Some(last) = &rotation.last_rotated else {
                continue;
            };
            let Ok(last) = chrono::DateTime::parse_from_rfc3339(last) else {
                continue;
            };
            let due = last + chrono::Duration::days(i64::from(days));
            if chrono::Utc::now() >= due {
                println!("- rotation due: {key} (since {})", due.to_rfc3339());
            }
        }
    }
    println!(
        "- ssh authentication: {}",
        if mdboard_actions::process::has_ssh_keys(loaded) {
            "enabled"
        } else {
            "disabled"
        }
    );
    if let Some(card) = active {
        println!(
            "- applicable orders: {}",
            loaded.applicable_orders(card).len()
        );
    }
    match active {
        Some(card) => {
            println!("- active batch: {} ({})", card.id, card.state);
            if let Some(vote) = loaded.latest_vote_for(&card.id) {
                println!("- active vote: {} ({:?})", vote.id, vote.status);
            }
        }
        None => println!("- active batch: none"),
    }
    match working {
        Some(card) => println!("- working task: {}", card.id),
        None => println!("- working task: none"),
    }
    if pending.is_empty() {
        println!("- meetings: none pending");
    } else {
        for meeting in &pending {
            println!(
                "- meeting: {} ({:?}, {:?})",
                meeting.id, meeting.kind, meeting.status
            );
        }
    }
    if blocked.is_empty() && errors.is_empty() {
        println!("- blocked/error: none");
    } else {
        for card in blocked {
            println!(
                "- blocked: {} — {}",
                card.id,
                card.blocked_reason.as_deref().unwrap_or("")
            );
        }
        for card in errors {
            println!("- error task: {}", card.id);
        }
    }
    Ok(())
}

fn print_process_brief(loaded: &ProcessLoaded, json: bool) -> Result<()> {
    let pending = loaded.pending_meetings();
    let active = loaded.active_batches().next();
    let working = loaded.working_tasks().next();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "active_batch": active.map(|card| card.id.clone()),
                "working_task": working.map(|card| card.id.clone()),
                "standing_orders": loaded.active_orders().len(),
                "governance": {
                    "guardian": loaded.policy.guardian.name,
                    "ssh_enabled": mdboard_actions::process::has_ssh_keys(loaded),
                    "stakeholders": loaded.policy.stakeholders.members.len(),
                    "allocated_seats": loaded.policy.stakeholders.total_seats(),
                    "max_seats": loaded.policy.stakeholders.max_seats,
                    "quorum_seats": loaded.policy.stakeholders.quorum_seats,
                },
                "needs_human": pending.iter().map(|meeting| meeting.id.clone()).collect::<Vec<_>>(),
                "roadmap": loaded.roadmap_count(),
                "limit": loaded.roadmap_limit(),
                "estimated_cost_eur": loaded.estimated_cost_eur(),
            }))?
        );
        return Ok(());
    }
    println!("# mdboard brief\n");
    if let Ok(branch) = mdboard_git::current_branch(&loaded.root)
        && branch.starts_with("sandbox/")
    {
        println!("- MODE: SANDBOX — playground only (`{branch}`)");
        println!("  return: mdboard sandbox leave");
    }
    match active {
        Some(card) => println!("- active: {} ({})", card.id, card.state),
        None => println!("- active: none"),
    }
    match working {
        Some(card) => println!("- working: {}", card.id),
        None => println!("- working: none"),
    }
    if pending.is_empty() {
        println!("- needs you: nothing pending");
    } else {
        for meeting in &pending {
            println!("- needs you: {} ({:?})", meeting.id, meeting.kind);
        }
    }
    println!(
        "- roadmap: {} of {} (€{:.4} estimated)",
        loaded.roadmap_count(),
        loaded.roadmap_limit(),
        loaded.estimated_cost_eur()
    );
    println!(
        "- governance: guardian `{}`, {}/{} seats, quorum {}",
        loaded.policy.guardian.name,
        loaded.policy.stakeholders.total_seats(),
        loaded.policy.stakeholders.max_seats,
        loaded.policy.stakeholders.quorum_seats
    );
    println!("- standing orders: {} active", loaded.active_orders().len());
    println!(
        "- ssh authentication: {}",
        if mdboard_actions::process::has_ssh_keys(loaded) {
            "enabled"
        } else {
            "disabled"
        }
    );
    Ok(())
}

fn print_process_actions(loaded: &ProcessLoaded, id: &str, json: bool) -> Result<()> {
    let card = loaded
        .find(id)
        .with_context(|| format!("card `{id}` not found"))?;
    let mut rows = Vec::new();
    for action in card.legal_actions() {
        let authority = mdboard_actions::process::action_authority(action, card.kind, card.state)
            .map_or_else(
                || "human".to_owned(),
                |authority| authority.as_str().to_owned(),
            );
        rows.push((action.as_str().to_owned(), authority));
    }
    if card.state == ProcessState::Handover {
        rows.push(("review".to_owned(), "agent".to_owned()));
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    println!(
        "{} is {} {} in `{}`\n",
        card.id,
        if card.kind.is_intent() { "an" } else { "a" },
        card.kind,
        card.state
    );
    if rows.is_empty() {
        println!("no legal actions");
    } else {
        for (action, authority) in rows {
            println!("- {action} ({authority})");
        }
    }
    Ok(())
}

fn print_process_history(loaded: &ProcessLoaded, id: &str, json: bool) -> Result<()> {
    let history = loaded.history(id);
    if json {
        println!("{}", serde_json::to_string_pretty(&history)?);
        return Ok(());
    }
    println!("history for {id}\n");
    if history.is_empty() {
        println!("no events");
    } else {
        for event in history {
            let from = event
                .from
                .map_or_else(|| "-".to_owned(), |state| state.to_string());
            let to = event
                .to
                .map_or_else(|| "-".to_owned(), |state| state.to_string());
            let auth = event
                .authorised_by
                .as_deref()
                .map_or(String::new(), |actor| format!(" (by {actor})"));
            println!(
                "- {} {}: {from} -> {to}{auth} [{}]",
                event.at, event.action, event.actor
            );
        }
    }
    Ok(())
}

fn run_task_command(ctx: &CliRef, command: TaskCommand) -> Result<()> {
    let actor = require_actor(ctx.actor)?;
    match command {
        TaskCommand::New {
            parent,
            title,
            body,
            depends_on,
        } => {
            let card = mdboard_actions::process::task_new(
                ctx.root,
                &actor,
                &parent,
                &title,
                &body,
                &depends_on,
            )?;
            print_process_card(&card, ctx.json)?;
            refresh_views(ctx)
        }
        TaskCommand::Work { id } => run_transition(ctx, &id, Action::Work, None, None, None, None),
        TaskCommand::Done { id, commit } => {
            run_transition(ctx, &id, Action::Done, None, None, None, Some(&commit))
        }
        TaskCommand::Fail { id, reason } => {
            run_transition(ctx, &id, Action::Fail, Some(&reason), None, None, None)
        }
        TaskCommand::Recover { id, auth } => run_transition(
            ctx,
            &id,
            Action::Recover,
            None,
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        ),
    }
}

fn run_upstream_command(ctx: &CliRef, command: UpstreamCommand) -> Result<()> {
    match command {
        UpstreamCommand::Start { base } => {
            let actor = require_actor(ctx.actor)?;
            require_process_role(ctx.root, &actor, &["owner", "agent"])?;
            let mut loaded = ProcessLoaded::load(ctx.root)?;
            if loaded.upstream.as_ref().is_some_and(|iteration| {
                iteration.status == mdboard_core::process::IterationStatus::Open
            }) {
                bail!("an upstream iteration is already open");
            }
            if !mdboard_git::is_clean(ctx.root)? {
                bail!(
                    "the working tree is dirty; commit or stash changes before starting a iteration"
                );
            }
            let current = mdboard_git::current_branch(ctx.root)?;
            if current != loaded.policy.branches.upstream {
                bail!(
                    "switch to the upstream branch `{}` before starting a iteration",
                    loaded.policy.branches.upstream
                );
            }
            if mdboard_git::branch_exists(ctx.root, &loaded.policy.branches.nightly)? {
                mdboard_git::merge_ff_only(ctx.root, &loaded.policy.branches.nightly)?;
            }
            let base = mdboard_git::head(ctx.root).unwrap_or(base);
            let iteration = loaded
                .upstream
                .as_ref()
                .map_or(1, |record| record.iteration + 1);
            let started_at = mdboard_driver::process::now();
            let lease_expires = (chrono::Utc::now() + chrono::Duration::hours(12))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let record = mdboard_core::process::UpstreamRecord {
                iteration,
                status: mdboard_core::process::IterationStatus::Open,
                base: base.clone(),
                writer: actor.clone(),
                lease_expires,
                started_at,
                merged_at: None,
            };
            loaded.write_upstream(&record)?;
            let event = mdboard_core::process::Event {
                id: loaded.next_event_id("upstream-start", None),
                at: mdboard_driver::process::now(),
                action: "upstream-start".to_owned(),
                card: None,
                from: None,
                to: None,
                actor: actor.clone(),
                executor: actor,
                authorised_by: None,
                authorization: None,
                reason: None,
                commit: None,
                metadata: Default::default(),
            };
            loaded.write_event(&event)?;
            println!("upstream iteration {iteration} is open at {base}");
        }
        UpstreamCommand::Status => {
            let loaded = ProcessLoaded::load(ctx.root)?;
            match loaded.upstream {
                Some(record) => println!(
                    "iteration {} {:?}, base {}, writer {}, lease until {}",
                    record.iteration,
                    record.status,
                    record.base,
                    record.writer,
                    record.lease_expires
                ),
                None => println!("no upstream iteration open"),
            }
        }
        UpstreamCommand::Renew { hours } => {
            let actor = require_actor(ctx.actor)?;
            if hours <= 0 {
                bail!("--hours must be greater than zero");
            }
            let mut loaded = ProcessLoaded::load(ctx.root)?;
            let mut record = loaded
                .upstream
                .clone()
                .context("no upstream iteration is open")?;
            if record.status != mdboard_core::process::IterationStatus::Open {
                bail!("the upstream iteration is not open");
            }
            if record.writer != actor && !actor_has_role(ctx.root, &actor, "owner")? {
                bail!("only the writer or the owner may renew the lease");
            }
            record.writer = actor;
            record.lease_expires = (chrono::Utc::now() + chrono::Duration::hours(hours))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            loaded.write_upstream(&record)?;
            println!("lease renewed until {}", record.lease_expires);
        }
        UpstreamCommand::Clear => {
            let actor = require_actor(ctx.actor)?;
            require_process_role(ctx.root, &actor, &["owner"])?;
            let mut loaded = ProcessLoaded::load(ctx.root)?;
            let mut record = loaded
                .upstream
                .clone()
                .context("no upstream iteration is open")?;
            record.lease_expires = mdboard_driver::process::now();
            loaded.write_upstream(&record)?;
            println!("writer lease cleared for iteration {}", record.iteration);
        }
        UpstreamCommand::Merge => {
            let actor = require_actor(ctx.actor)?;
            require_process_role(ctx.root, &actor, &["owner"])?;
            let mut loaded = ProcessLoaded::load(ctx.root)?;
            let record = loaded
                .upstream
                .clone()
                .context("no upstream iteration is open")?;
            if record.status != mdboard_core::process::IterationStatus::Open {
                bail!("the upstream iteration is not open");
            }
            if loaded.active_batches().next().is_some() {
                bail!("a batch is still in breakdown, doing or handover");
            }
            if loaded
                .cards
                .iter()
                .any(|card| card.state == ProcessState::Blocked)
            {
                bail!("a batch is blocked");
            }
            if !mdboard_git::is_clean(ctx.root)? {
                bail!("the working tree is dirty; commit or stash changes before merging");
            }
            let upstream = loaded.policy.branches.upstream.clone();
            let nightly = loaded.policy.branches.nightly.clone();
            if !mdboard_git::branch_exists(ctx.root, &nightly)? {
                mdboard_git::switch_create(ctx.root, &nightly)?;
            } else {
                mdboard_git::switch(ctx.root, &nightly)?;
            }
            let merge_result = mdboard_git::merge_no_ff(
                ctx.root,
                &upstream,
                &format!("Merge upstream iteration {}", record.iteration),
            );
            if let Err(error) = merge_result {
                let _ = write_conflict_report(
                    &loaded,
                    &format!("upstream iteration {} → {nightly}", record.iteration),
                    &error,
                );
                let _ = mdboard_git::switch(ctx.root, &upstream);
                return Err(error);
            }
            let merge_commit = mdboard_git::head(ctx.root)?;
            let mut closed = record.clone();
            closed.status = mdboard_core::process::IterationStatus::Merged;
            closed.merged_at = Some(mdboard_driver::process::now());
            loaded.write_upstream(&closed)?;
            let event = mdboard_core::process::Event {
                id: loaded.next_event_id("upstream-merge", None),
                at: mdboard_driver::process::now(),
                action: "upstream-merge".to_owned(),
                card: None,
                from: None,
                to: None,
                actor: actor.clone(),
                executor: actor,
                authorised_by: None,
                authorization: None,
                reason: None,
                commit: Some(merge_commit.clone()),
                metadata: Default::default(),
            };
            loaded.write_event(&event)?;
            refresh_views(ctx)?;
            if !mdboard_git::commit_all(
                ctx.root,
                &format!("Close upstream iteration {}", record.iteration),
            )? {
                bail!("could not commit the upstream iteration closure");
            }
            mdboard_git::switch(ctx.root, &upstream)?;
            // Bring the closure commit back to upstream so the next iteration starts clean.
            mdboard_git::merge_ff_only(ctx.root, &nightly)?;
            println!(
                "iteration {} merged into {nightly} at {merge_commit}",
                record.iteration
            );
        }
        UpstreamCommand::Release => {
            let actor = require_actor(ctx.actor)?;
            require_process_role(ctx.root, &actor, &["owner"])?;
            let loaded = ProcessLoaded::load(ctx.root)?;
            if !mdboard_git::is_clean(ctx.root)? {
                bail!("the working tree is dirty; commit or stash changes before releasing");
            }
            let nightly = loaded.policy.branches.nightly.clone();
            let stable = loaded.policy.branches.stable.clone();
            if !mdboard_git::branch_exists(ctx.root, &nightly)? {
                bail!("the nightly branch `{nightly}` does not exist");
            }
            if !mdboard_git::branch_exists(ctx.root, &stable)? {
                bail!("the stable branch `{stable}` does not exist");
            }
            let tag = format!("v{}", generated_version::VERSION);
            if mdboard_git::tag_exists(ctx.root, &tag)? {
                bail!("release tag `{tag}` already exists");
            }
            let nightly_commit = mdboard_git::rev_parse(ctx.root, &nightly)?;
            mdboard_git::switch(ctx.root, &stable)?;
            let merge_result = mdboard_git::merge_no_ff(
                ctx.root,
                &nightly,
                &format!("Release {}", generated_version::VERSION),
            );
            if let Err(error) = merge_result {
                let _ = write_conflict_report(
                    &loaded,
                    &format!("release {nightly} → {stable}"),
                    &error,
                );
                let _ = mdboard_git::switch(ctx.root, &nightly);
                return Err(error);
            }
            let merged_commit = mdboard_git::head(ctx.root)?;
            let releases = loaded.process_root().join("releases");
            std::fs::create_dir_all(&releases)?;
            let notes_path = releases.join(format!("{tag}.md"));
            let mut notes = format!(
                "# Release {tag}\n\n- Date: {}\n- Nightly: `{nightly_commit}`\n- Stable merge: `{merged_commit}`\n- Owner: {actor}\n\n## Completed batches\n\n",
                mdboard_driver::process::now()
            );
            let mut completed = 0;
            for card in loaded
                .cards
                .iter()
                .filter(|card| card.state == ProcessState::Completed)
            {
                completed += 1;
                let report = card.report.as_deref().unwrap_or("no report");
                notes.push_str(&format!(
                    "- {} — {} ([report]({report}))\n",
                    card.id, card.title
                ));
            }
            if completed == 0 {
                notes.push_str("- No completed batches in this release.\n");
            }
            std::fs::write(&notes_path, notes)?;
            if !mdboard_git::commit_all(ctx.root, &format!("Release notes {tag}"))? {
                bail!("release notes could not be committed");
            }
            let release_commit = mdboard_git::head(ctx.root)?;
            mdboard_git::tag(ctx.root, &tag)?;
            println!(
                "released {stable} from {nightly} at {release_commit} ({tag}); notes in {}",
                notes_path.display()
            );
            let _ = mdboard_git::switch(ctx.root, &loaded.policy.branches.upstream);
        }
    }
    refresh_views(ctx)?;
    Ok(())
}

fn write_conflict_report(
    loaded: &ProcessLoaded,
    context: &str,
    error: &anyhow::Error,
) -> Result<()> {
    let directory = loaded.process_root().join("reports");
    std::fs::create_dir_all(&directory)?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
    let path = directory.join(format!("conflict-{stamp}.md"));
    let content = format!(
        "# Merge conflict\n\n- Context: {context}\n- Time: {}\n- Working tree: {}\n\n## Error\n\n```text\n{error:#}\n```\n\nThe merge was aborted. A human must decide the resolution; no side was chosen automatically.\n",
        mdboard_driver::process::now(),
        loaded.root.display()
    );
    std::fs::write(&path, content)?;
    eprintln!("conflict report: {}", path.display());
    Ok(())
}

fn validate_commit_paths(class: CommitClass, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let is_generated = |path: &str| {
        matches!(
            path,
            "BOARD.md"
                | "ROADMAP.md"
                | "VERSION"
                | "TAG"
                | "BUILD"
                | "BUILD_DATE"
                | "RELEASE"
                | "CHANGELOG.md"
        ) || path.starts_with(".mdboard/site/")
    };
    let is_code = |path: &str| {
        path.starts_with("crates/")
            || path.starts_with("web/")
            || path.starts_with("sbin/")
            || path == "Cargo.toml"
            || path == "Cargo.lock"
            || path == "Dockerfile"
            || path.starts_with(".github/")
    };
    for path in paths {
        match class {
            CommitClass::Mdboard if is_code(path) => {
                bail!("mdboard identity cannot commit source code: {path}")
            }
            CommitClass::Agent if is_generated(path) => {
                bail!(
                    "agent identity cannot commit generated output: {path}; use --identity mdboard"
                )
            }
            CommitClass::Human if is_code(path) => {
                bail!("human identity cannot commit source code: {path}")
            }
            _ => {}
        }
    }
    Ok(())
}

fn commit_identity(entry: &mdboard_core::actor::Actor) -> (Option<String>, Option<String>) {
    let agent = entry.kind == mdboard_core::actor::ActorKind::Agent;
    (
        std::env::var("MDBOARD_AGENT_GIT_NAME")
            .ok()
            .or_else(|| entry.git_name.clone())
            .or_else(|| agent.then(|| "agent".to_owned())),
        std::env::var("MDBOARD_AGENT_GIT_EMAIL")
            .ok()
            .or_else(|| entry.git_email.clone())
            .or_else(|| agent.then(|| "no-replay@example.com".to_owned())),
    )
}

fn actors_path(root: &Path) -> Result<PathBuf> {
    let loaded = ProcessLoaded::load(root)?;
    Ok(loaded.process_root().join("actors.toml"))
}

fn actor_has_role(root: &Path, actor: &str, role: &str) -> Result<bool> {
    let actors = Actors::load_optional(&actors_path(root)?)?;
    Ok(actors.get(actor).is_some_and(|entry| entry.has_role(role)))
}

fn require_process_role(root: &Path, actor: &str, roles: &[&str]) -> Result<()> {
    let actors = Actors::load_optional(&actors_path(root)?)?;
    let entry = actors
        .get(actor)
        .with_context(|| format!("actor `{actor}` is not configured"))?;
    if entry.retired {
        bail!("actor `{actor}` is retired");
    }
    if !roles.iter().any(|role| entry.has_role(role)) {
        bail!("actor `{actor}` does not have the required role");
    }
    Ok(())
}

fn run_meeting_command(ctx: &CliRef, command: MeetingCommand) -> Result<()> {
    let actor = require_actor(ctx.actor)?;
    match command {
        MeetingCommand::List => {
            let loaded = ProcessLoaded::load(ctx.root)?;
            if ctx.json {
                println!("{}", serde_json::to_string_pretty(&loaded.meetings)?);
            } else if loaded.meetings.is_empty() {
                println!("no meetings");
            } else {
                for meeting in &loaded.meetings {
                    println!(
                        "{} {:?} {:?} — {}",
                        meeting.id, meeting.kind, meeting.status, meeting.trigger
                    );
                }
            }
        }
        MeetingCommand::Open {
            kind,
            batch,
            trigger,
        } => {
            let trigger = if trigger.trim().is_empty() {
                "on-demand meeting".to_owned()
            } else {
                trigger
            };
            let meeting =
                mdboard_actions::process::meeting_open(ctx.root, &actor, kind, &batch, &trigger)?;
            println!("opened {} ({:?}) for {}", meeting.id, meeting.kind, batch);
        }
        MeetingCommand::Question { command } => match command {
            QuestionCommand::Add { meeting, text } => {
                let meeting =
                    mdboard_actions::process::meeting_question(ctx.root, &actor, &meeting, &text)?;
                println!("question added to {}", meeting.id);
            }
            QuestionCommand::Resolve { meeting, question } => {
                let meeting = mdboard_actions::process::meeting_resolve(
                    ctx.root, &actor, &meeting, &question,
                )?;
                println!("resolved {} in {}", question, meeting.id);
            }
        },
        MeetingCommand::Resolve { meeting, question } => {
            let meeting =
                mdboard_actions::process::meeting_resolve(ctx.root, &actor, &meeting, &question)?;
            println!("resolved {} in {}", question, meeting.id);
        }
        MeetingCommand::Accept { meeting } => {
            let meeting = mdboard_actions::process::meeting_accept(ctx.root, &actor, &meeting)?;
            println!("accepted {}", meeting.id);
        }
        MeetingCommand::Defer { meeting, until } => {
            let meeting =
                mdboard_actions::process::meeting_defer(ctx.root, &actor, &meeting, &until)?;
            println!("deferred {} until {}", meeting.id, until);
        }
        MeetingCommand::Dismiss { meeting, reason } => {
            let meeting =
                mdboard_actions::process::meeting_dismiss(ctx.root, &actor, &meeting, &reason)?;
            println!("dismissed {}", meeting.id);
        }
    }
    refresh_views(ctx)?;
    Ok(())
}

fn parse_findings(raw: &[String]) -> Result<Vec<(String, String)>> {
    let mut findings = Vec::new();
    for value in raw {
        let (id, text) = value
            .split_once(':')
            .with_context(|| format!("finding `{value}` must be ID:TEXT"))?;
        findings.push((id.trim().to_owned(), text.trim().to_owned()));
    }
    Ok(findings)
}

fn parse_verdict(value: &str) -> std::result::Result<Verdict, String> {
    match value {
        "accepted" => Ok(Verdict::Accepted),
        "changes-required" => Ok(Verdict::ChangesRequired),
        other => Err(format!(
            "unknown verdict `{other}`; use accepted or changes-required"
        )),
    }
}

fn parse_meeting_kind(
    value: &str,
) -> std::result::Result<mdboard_core::process::MeetingKind, String> {
    match value {
        "consortium" => Ok(mdboard_core::process::MeetingKind::Consortium),
        "breakdown-clarification" => Ok(mdboard_core::process::MeetingKind::BreakdownClarification),
        "handover-review" => Ok(mdboard_core::process::MeetingKind::HandoverReview),
        "release-readiness" => Ok(mdboard_core::process::MeetingKind::ReleaseReadiness),
        "recovery" => Ok(mdboard_core::process::MeetingKind::Recovery),
        other => Err(format!("unknown meeting kind `{other}`")),
    }
}

fn main() -> ExitCode {
    if std::env::args_os()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
    {
        println!("{}", generated_version::banner());
        println!("{}", generated_version::qualified());
        println!("{}", generated_version::VERSION_STRING);
        return ExitCode::SUCCESS;
    }
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<u8> {
    if !matches!(&cli.command, Command::Init { .. }) {
        project_version::guard(&cli.root)?;
    }
    if !matches!(
        &cli.command,
        Command::Init { .. } | Command::Check { .. } | Command::Render { .. }
    ) {
        project_version::ensure_mdboard(&cli.root)?;
    }
    let ctx = CliRef {
        root: &cli.root,
        json: cli.json,
        actor: cli.actor.as_deref(),
    };
    match cli.command {
        Command::Init {
            dir,
            force,
            upstream,
            subscription,
            eur_per_million_tokens,
            roadmap_limit,
            init_git,
            agent_git_name,
            agent_git_email,
            no_prompt,
            ssh_key,
        } => init(
            &dir,
            force,
            upstream.as_deref(),
            subscription.as_deref(),
            eur_per_million_tokens,
            roadmap_limit,
            init_git,
            agent_git_name,
            agent_git_email,
            no_prompt,
            ssh_key.as_deref(),
        )?,
        Command::Update { check } => {
            update_check(&cli.root)?;
            if !check {
                println!("update is check-only in this build");
            }
        }
        Command::Status => {
            let loaded = ProcessLoaded::load(&cli.root)?;
            print_process_context(&loaded, cli.json)?;
            if let Some(notice) = project_version::release_notice(&cli.root) {
                eprintln!("{notice}");
            }
        }
        Command::Check { git } => {
            let loaded = ProcessLoaded::load(&cli.root)?;
            let findings = process_check(&loaded, git);
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&findings)?);
            } else if findings.is_empty() {
                println!("mdboard check: ok ({} cards)", loaded.cards.len());
            } else {
                for finding in &findings {
                    println!("{} {}: {}", finding.level, finding.code, finding.message);
                }
            }
            if findings.iter().any(|finding| finding.level == "error") {
                return Ok(1);
            }
        }
        Command::Migrate { to, yes } => {
            migrate_config(&cli.root, to, yes)?;
        }
        Command::Render { check } => {
            let loaded = ProcessLoaded::load(&cli.root)?;
            let views = process_views(&loaded);
            if check {
                let mut differences = Vec::new();
                for (path, content) in &views {
                    match std::fs::read_to_string(path) {
                        Ok(existing) if existing == *content => {}
                        Ok(_) => differences.push(path.display().to_string()),
                        Err(_) => differences.push(path.display().to_string()),
                    }
                }
                if differences.is_empty() {
                    println!("mdboard render --check: ok");
                } else {
                    for path in differences {
                        println!("stale: {path}");
                    }
                    return Ok(1);
                }
            } else {
                for (path, content) in &views {
                    std::fs::write(path, content)?;
                    println!("mdboard render: wrote {}", path.display());
                }
            }
        }
        Command::Watch => {
            mdboard_serve::watch(&cli.root)?;
        }
        Command::Serve {
            host,
            port,
            open,
            no_watch,
        } => {
            let options = ServeOptions {
                host,
                port,
                open,
                watch: !no_watch,
                actor: cli.actor.clone(),
            };
            mdboard_serve::serve(&cli.root, &options)?;
        }
        Command::Schema => {
            let kinds = Kind::ALL
                .iter()
                .map(|kind| {
                    serde_json::json!({
                        "kind": kind.as_str(),
                        "states": kind.states().iter().map(|state| state.as_str()).collect::<Vec<_>>(),
                    })
                })
                .collect::<Vec<_>>();
            let transitions = TRANSITIONS
                .iter()
                .map(|rule| {
                    serde_json::json!({
                        "action": rule.action.as_str(),
                        "from": rule.from.as_str(),
                        "to": rule.to.as_str(),
                        "from_kind": rule.from_kind.as_str(),
                        "to_kind": rule.to_kind.map(|kind| kind.as_str()),
                        "authority": rule.authority.as_str(),
                        "guard": rule.guard,
                    })
                })
                .collect::<Vec<_>>();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "schema": 2,
                    "kinds": kinds,
                    "transitions": transitions,
                }))?
            );
        }
        Command::Context => {
            let loaded = ProcessLoaded::load(&cli.root)?;
            print_process_context(&loaded, cli.json)?;
        }
        Command::Runbook => {
            let loaded = ProcessLoaded::load(ctx.root)?;
            println!("{}", mdboard_actions::process::RUNBOOK);
            let active = loaded.active_orders();
            if !active.is_empty() {
                println!("\n## Active standing orders\n");
                for order in active {
                    println!(
                        "### {} — {}\n\n{}\n",
                        order.id, order.title, order.instruction
                    );
                }
            }
        }
        Command::Brief => {
            let loaded = ProcessLoaded::load(ctx.root)?;
            print_process_brief(&loaded, cli.json)?;
        }
        Command::Actions { id } => {
            let loaded = ProcessLoaded::load(ctx.root)?;
            print_process_actions(&loaded, &id, cli.json)?;
        }
        Command::Attachment { command } => {
            let loaded = ProcessLoaded::load(ctx.root)?;
            match command {
                AttachmentCommand::List { id } => {
                    loaded.require(&id)?;
                    let files = loaded.attachment_files(&id)?;
                    let files = files
                        .iter()
                        .map(|path| relative_display(&loaded.root, path))
                        .collect::<Vec<_>>();
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&files)?);
                    } else {
                        for file in files {
                            println!("{file}");
                        }
                    }
                }
                AttachmentCommand::Path { id, name } => {
                    loaded.require(&id)?;
                    let path = loaded.attachment_path(&id, &name)?;
                    if !path.is_file() {
                        bail!("attachment `{name}` for {id} is missing");
                    }
                    println!("{}", relative_display(&loaded.root, &path));
                }
            }
        }
        Command::Obsolete { id, reason } => {
            run_transition(&ctx, &id, Action::Obsolete, Some(&reason), None, None, None)?
        }
        Command::ReopenTask { id, reason } => run_transition(
            &ctx,
            &id,
            Action::ReopenTask,
            Some(&reason),
            None,
            None,
            None,
        )?,
        Command::ActorRemove {
            actor_id,
            retire,
            delete,
            auth,
        } => {
            governance_authorization(ctx.actor, &auth)?;
            actor_remove(ctx.root, &actor_id, retire, delete)?;
            println!(
                "actor {actor_id} {}",
                if retire { "retired" } else { "deleted" }
            );
        }
        Command::Person { command } => match command {
            PersonCommand::Add {
                actor_id,
                display_name,
                role,
                kind,
                git_email,
                email,
                organisation,
                seats,
                until,
                guardian,
                co_guardian,
                auth,
            } => {
                governance_authorization(ctx.actor, &auth)?;
                person_add(
                    ctx.root,
                    &actor_id,
                    &display_name,
                    &role,
                    &kind,
                    git_email.as_deref(),
                    email.as_deref(),
                    organisation.as_deref(),
                    seats,
                    until.as_deref(),
                    guardian,
                    co_guardian,
                )?;
                println!("person {actor_id} added");
            }
        },
        Command::Archive { id, reason, auth } => {
            let card = mdboard_actions::process::archive_card(
                &cli.root,
                &require_actor(ctx.actor)?,
                &id,
                reason.as_deref(),
                auth.authorised_by.as_deref(),
                auth.authorization.as_deref(),
            )?;
            print_process_card(&card, cli.json)?;
        }
        Command::Unarchive { id, auth } => {
            let card = mdboard_actions::process::unarchive_card(
                &cli.root,
                &require_actor(ctx.actor)?,
                &id,
                auth.authorised_by.as_deref(),
                auth.authorization.as_deref(),
            )?;
            print_process_card(&card, cli.json)?;
        }
        Command::History { id } => {
            let loaded = ProcessLoaded::load(ctx.root)?;
            print_process_history(&loaded, &id, cli.json)?;
        }
        Command::New {
            kind,
            title,
            body,
            intent_owner,
        } => {
            let actor = require_actor(ctx.actor)?;
            let card = mdboard_actions::process::new_intent(
                &cli.root,
                &actor,
                kind.into(),
                &title,
                &body,
                &intent_owner,
            )?;
            print_process_card(&card, cli.json)?;
            refresh_views(&ctx)?;
        }
        Command::Next { batch } => {
            let loaded = ProcessLoaded::load(ctx.root)?;
            let card = loaded
                .find(&batch)
                .ok_or_else(|| anyhow::anyhow!("batch `{batch}` not found"))?;
            if card.kind != mdboard_core::process::Kind::Batch {
                bail!("`{batch}` is not a batch");
            }
            let tasks: Vec<_> = loaded
                .cards
                .iter()
                .filter(|task| task.parent.as_deref() == Some(card.id.as_str()))
                .collect();
            let next = tasks.iter().find(|task| {
                task.state == ProcessState::Pending
                    && task.depends_on.iter().all(|dependency| {
                        loaded.find(dependency).is_some_and(|parent| {
                            matches!(parent.state, ProcessState::Done | ProcessState::Obsolete)
                        })
                    })
            });
            let remaining: Vec<_> = tasks
                .iter()
                .filter(|task| {
                    task.state == ProcessState::Pending || task.state == ProcessState::Working
                })
                .map(|task| task.id.clone())
                .collect();
            let payload = serde_json::json!({
                "batch": card.id,
                "state": card.state,
                "next": next.map(|task| task.id.clone()),
                "remaining": remaining,
                "reason": if next.is_none() { Some("no ready task; handover or a blocker is next") } else { None },
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
            if next.is_none() {
                std::process::exit(1);
            }
        }
        Command::BatchStatus { batch } => {
            let loaded = ProcessLoaded::load(ctx.root)?;
            let card = loaded
                .find(&batch)
                .ok_or_else(|| anyhow::anyhow!("batch `{batch}` not found"))?;
            let tasks: Vec<_> = loaded
                .cards
                .iter()
                .filter(|task| task.parent.as_deref() == Some(card.id.as_str()))
                .collect();
            let count =
                |state: ProcessState| tasks.iter().filter(|task| task.state == state).count();
            let payload = serde_json::json!({
                "batch": card.id,
                "state": card.state,
                "total": tasks.len(),
                "done": count(ProcessState::Done),
                "pending": count(ProcessState::Pending),
                "working": count(ProcessState::Working),
                "blocked": count(ProcessState::Blocked),
                "obsolete": count(ProcessState::Obsolete),
                "canceled": count(ProcessState::Canceled),
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
        Command::QualityGate {
            card,
            passed,
            findings,
        } => {
            mdboard_actions::process::record_quality_gate(
                &cli.root,
                &require_actor(ctx.actor)?,
                &card,
                passed,
                &findings,
            )?;
            println!(
                "quality gate {} for {card}",
                if passed { "passed" } else { "failed" }
            );
        }
        Command::Sandbox { command } => match command {
            SandboxCommand::Create { name } => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                if !mdboard_git::is_clean(ctx.root)? {
                    bail!("sandbox create requires a clean working tree");
                }
                let current = mdboard_git::current_branch(ctx.root)?;
                if current != loaded.policy.branches.upstream {
                    bail!(
                        "sandbox create must start from `{}`",
                        loaded.policy.branches.upstream
                    );
                }
                let branch = format!("sandbox/{name}");
                let status = std::process::Command::new("git")
                    .current_dir(ctx.root)
                    .args(["switch", "-c", &branch, &loaded.policy.branches.upstream])
                    .status()?;
                if !status.success() {
                    bail!("cannot create sandbox `{branch}`");
                }
                println!("sandbox {branch} created");
            }
            SandboxCommand::Status => {
                let branch = mdboard_git::current_branch(ctx.root)?;
                if branch.starts_with("sandbox/") {
                    println!("MODE: SANDBOX — playground only");
                    println!("branch: {branch}");
                    println!("return: mdboard sandbox leave");
                } else {
                    println!("not in a sandbox (branch {branch})");
                }
            }
            SandboxCommand::Leave => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                if !mdboard_git::is_clean(ctx.root)? {
                    bail!("sandbox leave requires a clean working tree");
                }
                mdboard_git::switch(ctx.root, &loaded.policy.branches.upstream)?;
                println!("returned to {}", loaded.policy.branches.upstream);
            }
            SandboxCommand::Reset { name, yes } => {
                if !yes {
                    bail!("sandbox reset is destructive; pass --yes");
                }
                let loaded = ProcessLoaded::load(ctx.root)?;
                if !mdboard_git::is_clean(ctx.root)? {
                    bail!("sandbox reset requires a clean working tree");
                }
                let branch = format!("sandbox/{name}");
                mdboard_git::switch(ctx.root, &branch)?;
                let status = std::process::Command::new("git")
                    .current_dir(ctx.root)
                    .args(["reset", "--hard", &loaded.policy.branches.upstream])
                    .status()?;
                if !status.success() {
                    bail!("cannot reset `{branch}`");
                }
                println!("{branch} reset to {}", loaded.policy.branches.upstream);
            }
            SandboxCommand::Delete { name, force } => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                if !mdboard_git::is_clean(ctx.root)? {
                    bail!("sandbox delete requires a clean working tree");
                }
                let branch = format!("sandbox/{name}");
                mdboard_git::switch(ctx.root, &loaded.policy.branches.upstream)?;
                let mut cmd = std::process::Command::new("git");
                cmd.current_dir(ctx.root);
                if force {
                    cmd.args(["branch", "-D", &branch]);
                } else {
                    cmd.args(["branch", "-d", &branch]);
                }
                if !cmd.status()?.success() {
                    bail!("cannot delete `{branch}`; use --force if unmerged");
                }
                println!("deleted {branch}");
            }
        },
        Command::Rotation { command } => match command {
            RotationCommand::Next { key } => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                let rotations = loaded
                    .rotations
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("no rotations configured"))?;
                let rotation = rotations
                    .rotations
                    .get(&key)
                    .ok_or_else(|| anyhow::anyhow!("rotation `{key}` not found"))?;
                let participants = rotations.effective_participants(&key)?;
                if participants.is_empty() {
                    bail!("rotation `{key}` has no participants");
                }
                let next = match rotation.strategy {
                    mdboard_core::process::RotationStrategy::Manual => {
                        bail!("rotation `{key}` is manual; use `rotation set`")
                    }
                    mdboard_core::process::RotationStrategy::RoundRobin => {
                        let index = participants
                            .iter()
                            .position(|person| person.email.eq_ignore_ascii_case(&rotation.current))
                            .unwrap_or(0);
                        participants[(index + 1) % participants.len()]
                    }
                    mdboard_core::process::RotationStrategy::Random => {
                        let eligible: Vec<_> = participants
                            .iter()
                            .filter(|person| !person.email.eq_ignore_ascii_case(&rotation.current))
                            .collect();
                        if eligible.is_empty() {
                            participants[0]
                        } else {
                            let nanos = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |value| value.subsec_nanos() as usize);
                            eligible[nanos % eligible.len()]
                        }
                    }
                };
                let path = ctx.root.join(".mdboard/rotations.toml");
                let mut value: toml::Value = toml::from_str(&std::fs::read_to_string(&path)?)?;
                let entry = value
                    .get_mut("rotations")
                    .and_then(toml::Value::as_table_mut)
                    .and_then(|rotations| rotations.get_mut(&key))
                    .and_then(toml::Value::as_table_mut)
                    .ok_or_else(|| anyhow::anyhow!("rotation `{key}` not found in file"))?;
                entry.insert(
                    "current".to_owned(),
                    toml::Value::String(next.email.clone()),
                );
                entry.insert(
                    "last_rotated".to_owned(),
                    toml::Value::String(chrono::Utc::now().to_rfc3339()),
                );
                std::fs::write(&path, toml::to_string_pretty(&value)?)?;
                println!(
                    "{}: {} -> {} ({})",
                    key, rotation.current, next.email, next.name
                );
            }
            RotationCommand::Set { key, person } => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                let rotations = loaded
                    .rotations
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("no rotations configured"))?;
                let rotation = rotations
                    .rotations
                    .get(&key)
                    .ok_or_else(|| anyhow::anyhow!("rotation `{key}` not found"))?;
                let participants = rotations.effective_participants(&key)?;
                let next = participants
                    .iter()
                    .find(|candidate| candidate.email.eq_ignore_ascii_case(&person))
                    .ok_or_else(|| anyhow::anyhow!("`{person}` is not an effective participant"))?;
                let path = ctx.root.join(".mdboard/rotations.toml");
                let mut value: toml::Value = toml::from_str(&std::fs::read_to_string(&path)?)?;
                let entry = value
                    .get_mut("rotations")
                    .and_then(toml::Value::as_table_mut)
                    .and_then(|rotations| rotations.get_mut(&key))
                    .and_then(toml::Value::as_table_mut)
                    .ok_or_else(|| anyhow::anyhow!("rotation `{key}` not found in file"))?;
                entry.insert(
                    "current".to_owned(),
                    toml::Value::String(next.email.clone()),
                );
                entry.insert(
                    "last_rotated".to_owned(),
                    toml::Value::String(chrono::Utc::now().to_rfc3339()),
                );
                std::fs::write(&path, toml::to_string_pretty(&value)?)?;
                let mut metadata = std::collections::BTreeMap::new();
                metadata.insert("rotation".to_owned(), key.clone());
                metadata.insert("previous".to_owned(), rotation.current.clone());
                metadata.insert("holder".to_owned(), next.email.clone());
                metadata.insert("trigger".to_owned(), "manual".to_owned());
                let mut loaded = ProcessLoaded::load(ctx.root)?;
                let event = mdboard_core::process::Event {
                    id: loaded.next_event_id("rotation", None),
                    at: mdboard_driver::process::now(),
                    action: "rotation".to_owned(),
                    card: None,
                    from: None,
                    to: None,
                    actor: require_actor(ctx.actor)?,
                    executor: "agent".to_owned(),
                    authorised_by: None,
                    authorization: None,
                    reason: Some("manual set".to_owned()),
                    commit: None,
                    metadata,
                };
                loaded.write_event(&event)?;
                println!(
                    "{}: {} -> {} ({})",
                    key, rotation.current, next.email, next.name
                );
            }
            RotationCommand::Auto => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                if let Some(rotations) = &loaded.rotations {
                    let mut due = Vec::new();
                    for (key, rotation) in &rotations.rotations {
                        let Some(days) = rotation.auto_rotate_days else {
                            continue;
                        };
                        let Some(last) = &rotation.last_rotated else {
                            continue;
                        };
                        let Ok(last) = chrono::DateTime::parse_from_rfc3339(last) else {
                            continue;
                        };
                        if chrono::Utc::now() >= last + chrono::Duration::days(i64::from(days)) {
                            due.push(key.clone());
                        }
                    }
                    for key in due {
                        let mut command = std::process::Command::new(std::env::current_exe()?);
                        command.args([
                            "--root",
                            &ctx.root.to_string_lossy(),
                            "--actor",
                            "agent",
                            "rotation",
                            "next",
                            &key,
                        ]);
                        let status = command.status()?;
                        if !status.success() {
                            bail!("rotation `{key}` failed");
                        }
                    }
                } else {
                    println!("no rotations configured");
                }
            }
            RotationCommand::Due => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                if let Some(rotations) = &loaded.rotations {
                    for (key, rotation) in &rotations.rotations {
                        let Some(days) = rotation.auto_rotate_days else {
                            continue;
                        };
                        let Some(last) = &rotation.last_rotated else {
                            continue;
                        };
                        let Ok(last) = chrono::DateTime::parse_from_rfc3339(last) else {
                            continue;
                        };
                        let due = last + chrono::Duration::days(i64::from(days));
                        if chrono::Utc::now() >= due {
                            println!("{key} due since {}", due.to_rfc3339());
                        }
                    }
                } else {
                    println!("no rotations configured");
                }
            }
            RotationCommand::History { key } => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                let mut found = false;
                for event in loaded.events.iter().filter(|event| {
                    event.action == "rotation"
                        && event.metadata.get("rotation").map(String::as_str) == Some(key.as_str())
                }) {
                    found = true;
                    println!(
                        "{} — {} -> {}",
                        event.at,
                        event
                            .metadata
                            .get("previous")
                            .map(String::as_str)
                            .unwrap_or("?"),
                        event
                            .metadata
                            .get("holder")
                            .map(String::as_str)
                            .unwrap_or("?")
                    );
                }
                if !found {
                    println!("no history for rotation `{key}`");
                }
            }
            RotationCommand::Show { key } => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                let rotations = loaded
                    .rotations
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("no rotations configured"))?;
                let rotation = rotations
                    .rotations
                    .get(&key)
                    .ok_or_else(|| anyhow::anyhow!("rotation `{key}` not found"))?;
                println!("{} — {}", key, rotation.title);
                println!("{}", rotation.description);
                println!("strategy: {:?}", rotation.strategy);
                println!("current: {}", rotation.current);
                if let Some(days) = rotation.auto_rotate_days {
                    println!("auto_rotate_days: {days}");
                }
            }
            RotationCommand::List => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                if let Some(rotations) = &loaded.rotations {
                    for (key, rotation) in &rotations.rotations {
                        println!(
                            "{} — {} — current {}",
                            key, rotation.title, rotation.current
                        );
                    }
                } else {
                    println!("no rotations configured");
                }
            }
        },
        Command::Archived => {
            let loaded = ProcessLoaded::load(ctx.root)?;
            for card in loaded
                .cards
                .iter()
                .filter(|card| card.archived_at.is_some())
            {
                println!(
                    "{} — {} — archived {}",
                    card.id,
                    card.title,
                    card.archived_at.as_deref().unwrap_or("")
                );
            }
        }
        Command::HandoverPrompt { card } => {
            let loaded = ProcessLoaded::load(ctx.root)?;
            let branch = mdboard_git::current_branch(ctx.root)?;
            let dirty = mdboard_git::is_dirty(ctx.root)?;
            println!("Continue mdBoard in {}.", ctx.root.display());
            println!("\nStart by recovering state from the repository, not from memory:");
            println!(
                "  mdboard runbook\n  mdboard context\n  mdboard brief\n  git status --short && git log --oneline -12"
            );
            if let Some(card) = &card {
                println!(
                    "  mdboard actions '{card}'\n  cat .mdboard/cards/{}.md",
                    card.trim_start_matches('#')
                );
            }
            println!("\nBoard state");
            println!("- branch: {branch}");
            println!("- worktree: {}", if dirty { "dirty" } else { "clean" });
            println!(
                "- iteration: {}",
                loaded
                    .upstream
                    .as_ref()
                    .map_or("none".to_owned(), |cycle| format!(
                        "{} ({:?})",
                        cycle.iteration, cycle.status
                    ))
            );
            println!(
                "- roadmap: {} of {}",
                loaded.roadmap_count(),
                loaded.roadmap_limit()
            );
            if let Some(active) = loaded.active_batches().next() {
                println!("- active batch: {} ({})", active.id, active.state);
            }
            if let Some(working) = loaded.working_tasks().next() {
                println!("- working task: {}", working.id);
            }
            println!(
                "- context reserve: {}%",
                loaded.policy.context.handover_reserve_percent
            );
            println!("\nNext steps");
            if let Some(active) = loaded.active_batches().next() {
                println!(
                    "1. mdboard batch-status '{}'; continue the active batch.",
                    active.id
                );
            } else {
                println!("1. Review the open cards and choose the next action.");
            }
            println!("\nBuild/test notes");
            println!("- cargo fmt --all -- --check");
            println!("- cargo clippy --workspace --all-targets -- -D warnings");
            println!("- cargo test --workspace");
            println!("- mdboard check --git && mdboard render --check");
            println!("\nProcess constraints");
            println!("- mdBoard is the process: named actions only.");
            println!("- No merge, release or push; the owner does those.");
        }
        Command::Kinds => {
            for kind in mdboard_core::process::Kind::ALL {
                println!(
                    "{}{}",
                    kind.as_str(),
                    if kind.is_intent() {
                        ""
                    } else {
                        " (system-created)"
                    }
                );
            }
        }
        Command::Refine { id, auth } => run_transition(
            &ctx,
            &id,
            Action::Refine,
            None,
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::Pitch {
            id,
            force_quality_gate,
            reason,
            auth,
        } => {
            if force_quality_gate {
                mdboard_actions::process::override_quality_gate(
                    &cli.root,
                    &require_actor(ctx.actor)?,
                    &id,
                    reason.as_deref().unwrap_or("forced pitch"),
                )?;
            }
            run_transition(
                &ctx,
                &id,
                Action::Pitch,
                None,
                auth.authorised_by.as_deref(),
                auth.authorization.as_deref(),
                None,
            )?;
        }
        Command::Accept { id, auth } => run_transition(
            &ctx,
            &id,
            Action::AcceptIdea,
            None,
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::RejectIdea { id, reason, auth } => run_transition(
            &ctx,
            &id,
            Action::RejectIdea,
            Some(&reason),
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::Park { id, reason, auth } => run_transition(
            &ctx,
            &id,
            Action::Park,
            Some(&reason),
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::Reopen { id, reason, auth } => run_transition(
            &ctx,
            &id,
            Action::Reopen,
            Some(&reason),
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::Estimate { id, estimate } => {
            let actor = require_actor(ctx.actor)?;
            let card = mdboard_actions::process::estimate(
                &cli.root,
                &actor,
                &id,
                (
                    estimate.tokens_optimistic,
                    estimate.tokens_realistic,
                    estimate.tokens_pessimistic,
                ),
                (
                    estimate.agent_hours_optimistic,
                    estimate.agent_hours_realistic,
                    estimate.agent_hours_pessimistic,
                ),
                false,
                None,
                None,
                None,
            )?;
            print_process_card(&card, cli.json)?;
            refresh_views(&ctx)?;
        }
        Command::Reestimate {
            id,
            estimate,
            reason,
            auth,
        } => {
            let actor = require_actor(ctx.actor)?;
            let card = mdboard_actions::process::estimate(
                &cli.root,
                &actor,
                &id,
                (
                    estimate.tokens_optimistic,
                    estimate.tokens_realistic,
                    estimate.tokens_pessimistic,
                ),
                (
                    estimate.agent_hours_optimistic,
                    estimate.agent_hours_realistic,
                    estimate.agent_hours_pessimistic,
                ),
                true,
                Some(&reason),
                auth.authorised_by.as_deref(),
                auth.authorization.as_deref(),
            )?;
            print_process_card(&card, cli.json)?;
            refresh_views(&ctx)?;
        }
        Command::Roadmap { id, auth } => run_transition(
            &ctx,
            &id,
            Action::Roadmap,
            None,
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::Unroadmap { id, auth } => run_transition(
            &ctx,
            &id,
            Action::Unroadmap,
            None,
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::Breakdown { id, auth } => run_transition(
            &ctx,
            &id,
            Action::Breakdown,
            None,
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::AbortBreakdown { id, reason, auth } => run_transition(
            &ctx,
            &id,
            Action::AbortBreakdown,
            Some(&reason),
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::Start { id, auth } => run_transition(
            &ctx,
            &id,
            Action::Start,
            None,
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::Handover { id } => {
            run_transition(&ctx, &id, Action::Handover, None, None, None, None)?
        }
        Command::Commit {
            message,
            identity,
            as_actor,
            auth,
        } => {
            let actor = require_actor(ctx.actor)?;
            // Loading the session enforces the open iteration and writer lease.
            let _session = mdboard_actions::process::Session::load(ctx.root, &actor)?;
            if message.trim().is_empty() {
                bail!("--message must not be empty");
            }
            let actors = Actors::load_optional(&actors_path(ctx.root)?)?;
            let executing_actor = actors
                .get(&actor)
                .with_context(|| format!("unknown executing actor `{actor}`"))?;
            let explicit_identity = identity.is_some();
            let class = identity.unwrap_or_else(|| {
                if as_actor.is_some()
                    || executing_actor.kind == mdboard_core::actor::ActorKind::Human
                {
                    CommitClass::Human
                } else {
                    CommitClass::Agent
                }
            });

            let (author, author_name, author_email) = match class {
                CommitClass::Mdboard => (
                    "mdboard".to_owned(),
                    Some("mdboard".to_owned()),
                    Some("no-reply@mdboard.io".to_owned()),
                ),
                CommitClass::Human => {
                    let author = as_actor.unwrap_or_else(|| actor.clone());
                    let author_actor = actors
                        .get(&author)
                        .with_context(|| format!("unknown human author `{author}`"))?
                        .clone();
                    if author_actor.kind != mdboard_core::actor::ActorKind::Human {
                        bail!("human identity requires a human actor");
                    }
                    if author != actor {
                        if executing_actor.kind == mdboard_core::actor::ActorKind::Human {
                            bail!("a human actor may only commit as themselves");
                        }
                        let statement = auth.authorization.as_deref().unwrap_or("").trim();
                        if auth.authorised_by.as_deref() != Some(author.as_str())
                            || statement.is_empty()
                        {
                            bail!("human identity requires --authorised-by and --authorization");
                        }
                    }
                    let (name, email) = commit_identity(&author_actor);
                    if name.is_none() || email.is_none() {
                        bail!("configure git_name and git_email for human actor `{author}`");
                    }
                    (author, name, email)
                }
                CommitClass::Agent => {
                    if executing_actor.kind != mdboard_core::actor::ActorKind::Agent {
                        bail!("agent identity requires an agent executing actor");
                    }
                    if as_actor.is_some() {
                        bail!("--as is only valid with human identity");
                    }
                    let (name, email) = commit_identity(executing_actor);
                    (actor.clone(), name, email)
                }
            };

            mdboard_git::stage_all(ctx.root)?;
            if explicit_identity {
                let staged = mdboard_git::staged_paths(ctx.root)?;
                validate_commit_paths(class, &staged)?;
            }

            let (committer_name, committer_email) = if class == CommitClass::Mdboard {
                (
                    Some("mdboard".to_owned()),
                    Some("no-reply@mdboard.io".to_owned()),
                )
            } else {
                commit_identity(executing_actor)
            };
            let mut commit_message = message.trim().to_owned();
            if class == CommitClass::Mdboard {
                commit_message.push_str("\n\nGenerated-by: mdboard");
            } else if author != actor {
                let statement = auth
                    .authorization
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .replace('\n', " ");
                commit_message.push_str(&format!(
                    "\n\nAuthorised-by: {author}\nAuthorization: {statement}\nPerformed-by: {actor}"
                ));
            }

            if !mdboard_git::commit_all_with_identities(
                ctx.root,
                &commit_message,
                Some((author_name.as_deref(), author_email.as_deref())),
                Some((committer_name.as_deref(), committer_email.as_deref())),
            )? {
                println!("nothing to commit");
            } else {
                let commit = mdboard_git::head(ctx.root)?;
                println!("committed {commit}");
            }
        }
        Command::Work { id } => run_transition(&ctx, &id, Action::Work, None, None, None, None)?,
        Command::Done { id, commit } => {
            run_transition(&ctx, &id, Action::Done, None, None, None, Some(&commit))?
        }
        Command::Fail { id, reason } => {
            run_transition(&ctx, &id, Action::Fail, Some(&reason), None, None, None)?
        }
        Command::Recover { id, auth } => run_transition(
            &ctx,
            &id,
            Action::Recover,
            None,
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::Task { command } => run_task_command(&ctx, command)?,
        Command::Vote { command } => match command {
            VoteCommand::Open { id, auth } => {
                let vote = mdboard_actions::process::open_vote(
                    &cli.root,
                    &require_actor(ctx.actor)?,
                    &id,
                    auth.authorised_by.as_deref(),
                    auth.authorization.as_deref(),
                )?;
                print_vote(&vote, cli.json)?;
            }
            VoteCommand::Record { id, email, choice } => {
                let vote = mdboard_actions::process::record_vote(
                    &cli.root,
                    &require_actor(ctx.actor)?,
                    &id,
                    &email,
                    choice.into(),
                )?;
                print_vote(&vote, cli.json)?;
            }
            VoteCommand::Veto { id, reason, auth } => {
                let vote = mdboard_actions::process::veto_vote(
                    &cli.root,
                    &require_actor(ctx.actor)?,
                    &id,
                    &reason,
                    auth.authorised_by.as_deref(),
                    auth.authorization.as_deref(),
                )?;
                print_vote(&vote, cli.json)?;
            }
            VoteCommand::Resolve { id, tie_break } => {
                let vote = mdboard_actions::process::resolve_vote(
                    &cli.root,
                    &require_actor(ctx.actor)?,
                    &id,
                    tie_break.map(Into::into),
                )?;
                print_vote(&vote, cli.json)?;
            }
            VoteCommand::Unresolved { id, reason, auth } => {
                let vote = mdboard_actions::process::close_vote_unresolved(
                    &cli.root,
                    &require_actor(ctx.actor)?,
                    &id,
                    &reason,
                    auth.authorised_by.as_deref(),
                    auth.authorization.as_deref(),
                )?;
                print_vote(&vote, cli.json)?;
            }
            VoteCommand::Show { id } => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                let vote = loaded
                    .latest_vote_for(&id)
                    .ok_or_else(|| anyhow::anyhow!("no vote for {id}"))?;
                print_vote(vote, cli.json)?;
            }
        },
        Command::Stakeholder { command } => match command {
            StakeholderCommand::List => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                for member in &loaded.policy.stakeholders.members {
                    println!(
                        "{} <{}> · {} seats · until {}",
                        member.name, member.email, member.seats, member.seat_until
                    );
                }
            }
            StakeholderCommand::Add {
                name,
                email,
                organisation,
                seats,
                until,
                auth,
            } => {
                governance_authorization(ctx.actor, &auth)?;
                stakeholder_add(
                    ctx.root,
                    &name,
                    &email,
                    organisation.as_deref(),
                    seats,
                    &until,
                )?;
                println!("stakeholder {email} added");
            }
            StakeholderCommand::Remove { email, auth } => {
                governance_authorization(ctx.actor, &auth)?;
                stakeholder_remove(ctx.root, &email)?;
                println!("stakeholder {email} removed");
            }
            StakeholderCommand::Renew { email, until, auth } => {
                governance_authorization(ctx.actor, &auth)?;
                let loaded = edit_config(ctx.root, |table| {
                    let stakeholders = table
                        .get_mut("stakeholders")
                        .and_then(toml::Value::as_table_mut)
                        .ok_or_else(|| anyhow::anyhow!("missing [stakeholders]"))?;
                    let term_days = stakeholders
                        .get("seat_term_days")
                        .and_then(toml::Value::as_integer)
                        .unwrap_or(365);
                    let members = stakeholders
                        .get_mut("members")
                        .and_then(toml::Value::as_array_mut)
                        .ok_or_else(|| anyhow::anyhow!("missing stakeholders.members"))?;
                    let member = members
                        .iter_mut()
                        .find(|member| {
                            member
                                .get("email")
                                .and_then(toml::Value::as_str)
                                .is_some_and(|value| value.eq_ignore_ascii_case(&email))
                        })
                        .ok_or_else(|| anyhow::anyhow!("stakeholder `{email}` not found"))?;
                    let today = chrono::Utc::now().date_naive();
                    let until = match until {
                        Some(value) => value,
                        None => (today + chrono::Duration::days(term_days)).to_string(),
                    };
                    let parsed = chrono::NaiveDate::parse_from_str(&until, "%Y-%m-%d")
                        .with_context(|| format!("invalid date `{until}`; use YYYY-MM-DD"))?;
                    if parsed <= today {
                        bail!("new seat expiry must be in the future");
                    }
                    let member = member
                        .as_table_mut()
                        .ok_or_else(|| anyhow::anyhow!("stakeholder entry is not a table"))?;
                    member.insert("seat_until".to_owned(), toml::Value::String(until.clone()));
                    Ok(())
                })?;
                let member = loaded
                    .policy
                    .stakeholders
                    .members
                    .iter()
                    .find(|member| member.email.eq_ignore_ascii_case(&email))
                    .expect("renewed member exists");
                println!(
                    "renewed {} <{}> until {}",
                    member.name, member.email, member.seat_until
                );
            }
        },
        Command::Guardian { command } => match command {
            GuardianCommand::Show => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                let guardian = &loaded.policy.guardian;
                println!(
                    "{} <{}> {}",
                    guardian.name, guardian.email, guardian.organisation
                );
                if let Some(fingerprint) = &guardian.ssh_fingerprint {
                    println!("ssh fingerprint: {fingerprint}");
                }
            }
            GuardianCommand::Transfer {
                name,
                email,
                organisation,
                ssh_key,
                ssh_fingerprint,
                auth,
            } => {
                governance_authorization(ctx.actor, &auth)?;
                if ssh_key.is_some() != ssh_fingerprint.is_some() {
                    bail!("--ssh-key and --ssh-fingerprint must be provided together");
                }
                let loaded = edit_config(ctx.root, |table| {
                    let guardian = table
                        .get_mut("guardian")
                        .and_then(toml::Value::as_table_mut)
                        .ok_or_else(|| anyhow::anyhow!("missing [guardian]"))?;
                    guardian.insert("name".to_owned(), toml::Value::String(name.clone()));
                    guardian.insert("email".to_owned(), toml::Value::String(email.clone()));
                    guardian.insert(
                        "organisation".to_owned(),
                        toml::Value::String(organisation.clone().unwrap_or_default()),
                    );
                    set_optional(guardian, "ssh_key", ssh_key.as_deref());
                    set_optional(guardian, "ssh_fingerprint", ssh_fingerprint.as_deref());
                    Ok(())
                })?;
                println!(
                    "guardian transferred to {} <{}>",
                    loaded.policy.guardian.name, loaded.policy.guardian.email
                );
            }
            GuardianCommand::CoList => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                for co in &loaded.policy.guardian.co_guardians {
                    println!("{} <{}> {}", co.name, co.email, co.organisation);
                }
            }
            GuardianCommand::CoAdd {
                name,
                email,
                organisation,
                ssh_key,
                ssh_fingerprint,
                auth,
            } => {
                governance_authorization(ctx.actor, &auth)?;
                if ssh_key.is_some() != ssh_fingerprint.is_some() {
                    bail!("--ssh-key and --ssh-fingerprint must be provided together");
                }
                let loaded = edit_config(ctx.root, |table| {
                    let guardian = table
                        .get_mut("guardian")
                        .and_then(toml::Value::as_table_mut)
                        .ok_or_else(|| anyhow::anyhow!("missing [guardian]"))?;
                    let co_guardians = guardian
                        .entry("co_guardians".to_owned())
                        .or_insert_with(|| toml::Value::Array(Vec::new()))
                        .as_array_mut()
                        .ok_or_else(|| anyhow::anyhow!("guardian.co_guardians is not an array"))?;
                    if co_guardians.iter().any(|co| {
                        co.get("email")
                            .and_then(toml::Value::as_str)
                            .is_some_and(|value| value.eq_ignore_ascii_case(&email))
                    }) {
                        bail!("a co-guardian with `{email}` already exists");
                    }
                    let mut co = toml::map::Map::new();
                    co.insert("name".to_owned(), toml::Value::String(name.clone()));
                    co.insert("email".to_owned(), toml::Value::String(email.clone()));
                    co.insert(
                        "organisation".to_owned(),
                        toml::Value::String(organisation.clone().unwrap_or_default()),
                    );
                    set_optional(&mut co, "ssh_key", ssh_key.as_deref());
                    set_optional(&mut co, "ssh_fingerprint", ssh_fingerprint.as_deref());
                    co_guardians.push(toml::Value::Table(co));
                    Ok(())
                })?;
                println!("added co-guardian {name} <{email}>");
                let _ = loaded;
            }
            GuardianCommand::CoRemove { email, auth } => {
                governance_authorization(ctx.actor, &auth)?;
                let loaded = edit_config(ctx.root, |table| {
                    let guardian = table
                        .get_mut("guardian")
                        .and_then(toml::Value::as_table_mut)
                        .ok_or_else(|| anyhow::anyhow!("missing [guardian]"))?;
                    let co_guardians = guardian
                        .get_mut("co_guardians")
                        .and_then(toml::Value::as_array_mut)
                        .ok_or_else(|| anyhow::anyhow!("guardian.co_guardians is not an array"))?;
                    let before = co_guardians.len();
                    co_guardians.retain(|co| {
                        !co.get("email")
                            .and_then(toml::Value::as_str)
                            .is_some_and(|value| value.eq_ignore_ascii_case(&email))
                    });
                    if co_guardians.len() == before {
                        bail!("co-guardian `{email}` not found");
                    }
                    Ok(())
                })?;
                println!("removed co-guardian {email}");
                let _ = loaded;
            }
            GuardianCommand::SshAdd { email, key, auth } => {
                governance_authorization(ctx.actor, &auth)?;
                let output = std::process::Command::new("ssh-keygen")
                    .args(["-lf"])
                    .arg(&key)
                    .output()
                    .context("cannot run ssh-keygen")?;
                if !output.status.success() {
                    bail!(
                        "invalid SSH public key: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                }
                let text = String::from_utf8_lossy(&output.stdout);
                let fingerprint = text
                    .split_whitespace()
                    .nth(1)
                    .ok_or_else(|| anyhow::anyhow!("cannot read SSH fingerprint"))?
                    .to_owned();
                let public_key = std::fs::read_to_string(&key)
                    .with_context(|| format!("cannot read {}", key.display()))?
                    .trim()
                    .to_owned();
                edit_config(ctx.root, |table| {
                    let guardian = table
                        .get_mut("guardian")
                        .and_then(toml::Value::as_table_mut)
                        .ok_or_else(|| anyhow::anyhow!("missing [guardian]"))?;
                    if guardian
                        .get("email")
                        .and_then(toml::Value::as_str)
                        .is_some_and(|value| value.eq_ignore_ascii_case(&email))
                    {
                        guardian.insert(
                            "ssh_key".to_owned(),
                            toml::Value::String(public_key.clone()),
                        );
                        guardian.insert(
                            "ssh_fingerprint".to_owned(),
                            toml::Value::String(fingerprint.clone()),
                        );
                        return Ok(());
                    }
                    let co_guardians = guardian
                        .get_mut("co_guardians")
                        .and_then(toml::Value::as_array_mut)
                        .ok_or_else(|| anyhow::anyhow!("guardian.co_guardians is not an array"))?;
                    let co = co_guardians
                        .iter_mut()
                        .find(|co| {
                            co.get("email")
                                .and_then(toml::Value::as_str)
                                .is_some_and(|value| value.eq_ignore_ascii_case(&email))
                        })
                        .and_then(toml::Value::as_table_mut)
                        .ok_or_else(|| {
                            anyhow::anyhow!("guardian/co-guardian `{email}` not found")
                        })?;
                    co.insert(
                        "ssh_key".to_owned(),
                        toml::Value::String(public_key.clone()),
                    );
                    co.insert(
                        "ssh_fingerprint".to_owned(),
                        toml::Value::String(fingerprint.clone()),
                    );
                    Ok(())
                })?;
                println!("SSH key set for {email}: {fingerprint}");
            }
            GuardianCommand::SshRemove { email, auth } => {
                governance_authorization(ctx.actor, &auth)?;
                edit_config(ctx.root, |table| {
                    let guardian = table
                        .get_mut("guardian")
                        .and_then(toml::Value::as_table_mut)
                        .ok_or_else(|| anyhow::anyhow!("missing [guardian]"))?;
                    if guardian
                        .get("email")
                        .and_then(toml::Value::as_str)
                        .is_some_and(|value| value.eq_ignore_ascii_case(&email))
                    {
                        guardian.remove("ssh_key");
                        guardian.remove("ssh_fingerprint");
                        return Ok(());
                    }
                    let co_guardians = guardian
                        .get_mut("co_guardians")
                        .and_then(toml::Value::as_array_mut)
                        .ok_or_else(|| anyhow::anyhow!("guardian.co_guardians is not an array"))?;
                    let co = co_guardians
                        .iter_mut()
                        .find(|co| {
                            co.get("email")
                                .and_then(toml::Value::as_str)
                                .is_some_and(|value| value.eq_ignore_ascii_case(&email))
                        })
                        .and_then(toml::Value::as_table_mut)
                        .ok_or_else(|| {
                            anyhow::anyhow!("guardian/co-guardian `{email}` not found")
                        })?;
                    co.remove("ssh_key");
                    co.remove("ssh_fingerprint");
                    Ok(())
                })?;
                println!("SSH key removed for {email}");
            }
            GuardianCommand::SshShow => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                let guardian = &loaded.policy.guardian;
                println!(
                    "{} <{}> {}",
                    guardian.name,
                    guardian.email,
                    guardian.ssh_fingerprint.as_deref().unwrap_or("no SSH key")
                );
                for co in &guardian.co_guardians {
                    println!(
                        "{} <{}> {}",
                        co.name,
                        co.email,
                        co.ssh_fingerprint.as_deref().unwrap_or("no SSH key")
                    );
                }
            }
            GuardianCommand::SshChallenge { card } => {
                let nonce = mdboard_actions::process::ssh_challenge(
                    &cli.root,
                    &require_actor(ctx.actor)?,
                    &card,
                )?;
                println!("{nonce}");
                eprintln!(
                    "Sign the nonce file with: ssh-keygen -Y sign -f ~/.ssh/id_ed25519 -n mdboard <file>"
                );
            }
            GuardianCommand::SshVerify {
                card,
                nonce,
                signature,
                email,
            } => {
                mdboard_actions::process::verify_ssh_signature(
                    &cli.root,
                    &require_actor(ctx.actor)?,
                    &card,
                    &nonce,
                    &signature,
                    &email,
                )?;
                println!("SSH signature verified for {card} by {email}");
            }
        },
        Command::Order { command } => match command {
            OrderCommand::List => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                for order in &loaded.orders {
                    println!("{} — {:?} — {}", order.id, order.status, order.title);
                }
            }
            OrderCommand::Show { id } => {
                let loaded = ProcessLoaded::load(ctx.root)?;
                let order = loaded
                    .orders
                    .iter()
                    .find(|order| order.id == id)
                    .ok_or_else(|| anyhow::anyhow!("order `{id}` not found"))?;
                print_order(order, cli.json)?;
            }
            OrderCommand::Propose {
                title,
                instruction,
                rationale,
                consequences,
                project,
                kinds,
                states,
                cards,
                from,
                until,
                auth,
            } => {
                let scope = build_order_scope(project, &kinds, &states, &cards, from, until)?;
                let order = mdboard_actions::process::propose_order(
                    &cli.root,
                    &require_actor(ctx.actor)?,
                    &title,
                    &instruction,
                    scope,
                    &rationale,
                    &consequences,
                    auth.authorised_by.as_deref(),
                    auth.authorization.as_deref(),
                )?;
                print_order(&order, cli.json)?;
            }
            OrderCommand::Add { id, auth } => {
                let order = mdboard_actions::process::add_order(
                    &cli.root,
                    &require_actor(ctx.actor)?,
                    &id,
                    auth.authorised_by.as_deref(),
                    auth.authorization.as_deref(),
                )?;
                print_order(&order, cli.json)?;
            }
            OrderCommand::Amend {
                id,
                instruction,
                reason,
                auth,
            } => {
                let order = mdboard_actions::process::amend_order(
                    &cli.root,
                    &require_actor(ctx.actor)?,
                    &id,
                    &instruction,
                    &reason,
                    auth.authorised_by.as_deref(),
                    auth.authorization.as_deref(),
                )?;
                print_order(&order, cli.json)?;
            }
            OrderCommand::Retire { id, reason, auth } => {
                let order = mdboard_actions::process::retire_order(
                    &cli.root,
                    &require_actor(ctx.actor)?,
                    &id,
                    &reason,
                    auth.authorised_by.as_deref(),
                    auth.authorization.as_deref(),
                )?;
                print_order(&order, cli.json)?;
            }
        },
        Command::Mirror {
            profile,
            to,
            branch,
            dry_run,
            push,
            force,
            confirm_first_push,
        } => mirror::run(
            ctx.root,
            &mirror::MirrorOptions {
                profile,
                to,
                branch,
                dry_run,
                push,
                force,
                confirm_first_push,
            },
        )?,
        Command::Review {
            batch,
            reviewer,
            authored_by,
            verdict,
            findings,
            round,
        } => {
            if let Ok(branch) = mdboard_git::current_branch(ctx.root)
                && branch.starts_with("sandbox/")
            {
                bail!("reviews are refused in a sandbox; return to upstream first");
            }
            let actor = require_actor(ctx.actor)?;
            let findings = parse_findings(&findings)?;
            let review = mdboard_actions::process::review(
                &cli.root,
                &actor,
                &batch,
                &reviewer,
                &authored_by,
                verdict,
                &findings,
                round,
            )?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&review)?);
            } else {
                println!(
                    "review {} recorded for {}: {:?}",
                    review.id, review.batch, review.verdict
                );
            }
            refresh_views(&ctx)?;
        }
        Command::RequestChanges { id, reason, auth } => run_transition(
            &ctx,
            &id,
            Action::RequestChanges,
            Some(&reason),
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::Complete { id, auth } => run_transition(
            &ctx,
            &id,
            Action::Complete,
            None,
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::Block { id, reason, auth } => run_transition(
            &ctx,
            &id,
            Action::Block,
            Some(&reason),
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::Unblock { id, auth } => run_transition(
            &ctx,
            &id,
            Action::Unblock,
            None,
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::Cancel { id, reason, auth } => run_transition(
            &ctx,
            &id,
            Action::Cancel,
            Some(&reason),
            auth.authorised_by.as_deref(),
            auth.authorization.as_deref(),
            None,
        )?,
        Command::Upstream { command } => run_upstream_command(&ctx, command)?,
        Command::Meeting { command } => run_meeting_command(&ctx, command)?,
        Command::Plugin { command } => run_plugin_command(&ctx, command)?,
        Command::Stats => {
            let loaded = ProcessLoaded::load(&cli.root)?;
            print_process_context(&loaded, cli.json)?;
        }
        Command::Whoami => {
            let loaded = ProcessLoaded::load(&cli.root)?;
            let actors = Actors::load_optional(&loaded.process_root().join("actors.toml"))?;
            if actors.actors.is_empty() {
                println!(
                    "no actors configured (add {}/actors.toml)",
                    loaded.paths.root
                );
            } else if cli.json {
                println!("{}", serde_json::to_string_pretty(&actors)?);
            } else {
                for (id, actor) in &actors.actors {
                    println!(
                        "{id}: {:?} — {} [{}]",
                        actor.kind,
                        actor.display_name,
                        actor.roles.join(", ")
                    );
                }
            }
        }
    }
    Ok(0)
}

fn ensure_upstream_checkout(dir: &Path, upstream: &str) -> Result<()> {
    let head = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(dir)
        .output()
        .context("cannot run git rev-parse")?;
    if head.status.success() {
        return Ok(());
    }
    let current = std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(dir)
        .output()
        .context("cannot run git branch")?;
    if String::from_utf8_lossy(&current.stdout).trim() == upstream {
        return Ok(());
    }
    let status = std::process::Command::new("git")
        .args(["switch", "-C", upstream])
        .current_dir(dir)
        .status()
        .context("cannot switch to the upstream branch")?;
    if !status.success() {
        bail!("cannot create the upstream branch `{upstream}`");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn update_check(root: &Path) -> Result<()> {
    use mdboard_core::version::Version;
    let project = std::fs::read_to_string(root.join("VERSION"))?;
    let project = project
        .trim()
        .parse::<Version>()
        .map_err(anyhow::Error::msg)?;
    let installed = env!("CARGO_PKG_VERSION")
        .parse::<Version>()
        .map_err(anyhow::Error::msg)?;
    let tag = std::fs::read_to_string(root.join("TAG")).unwrap_or_default();
    let tag = tag.trim();
    let stable = std::process::Command::new("git")
        .current_dir(root)
        .args(["show", "origin/stable:VERSION"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse::<Version>()
                .map_err(anyhow::Error::msg)
        })
        .transpose()?;
    println!("installed: {installed}");
    println!("project:   {project}");
    println!(
        "stable:    {}",
        stable
            .clone()
            .map_or("unknown".to_owned(), |value| value.to_string())
    );
    println!("tag:       {tag}");
    let stable = stable.context("cannot read origin/stable:VERSION; fetch stable first")?;
    if installed == stable && project == stable {
        println!("up to date");
    } else if stable > installed {
        println!("update available from stable; owner approval required");
    } else if installed > stable {
        println!("stable is behind the installed build; do not downgrade");
        if (tag == "dev" || tag == "preview") && project < installed {
            println!("preview build may migrate the project; ask the owner");
        } else if project > stable {
            println!("inconsistent state: flag it");
        }
    } else {
        println!("no action");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn init(
    dir: &Path,
    force: bool,
    upstream: Option<&str>,
    subscription: Option<&str>,
    eur_per_million_tokens: Option<f64>,
    roadmap_limit: u32,
    init_git: bool,
    mut agent_git_name: Option<String>,
    mut agent_git_email: Option<String>,
    no_prompt: bool,
    ssh_key: Option<&Path>,
) -> Result<()> {
    if agent_git_name.is_none() || agent_git_email.is_none() {
        let interactive = std::io::stdin().is_terminal() && !no_prompt;
        eprintln!(
            "agent commits default to `{} <{}>`",
            agent_git_name.as_deref().unwrap_or("agent"),
            agent_git_email
                .as_deref()
                .unwrap_or("no-replay@example.com")
        );
        if interactive {
            if agent_git_name.is_none() {
                eprint!("agent git name (blank = default): ");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                agent_git_name = Some(input.trim().to_owned()).filter(|value| !value.is_empty());
            }
            if agent_git_email.is_none() {
                eprint!("agent git e-mail (blank = default): ");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                agent_git_email = Some(input.trim().to_owned()).filter(|value| !value.is_empty());
            }
        }
    }
    if roadmap_limit == 0 {
        bail!("--roadmap-limit must be greater than zero");
    }
    std::fs::create_dir_all(dir)?;
    ensure_git_repository(dir, init_git)?;
    let upstream = select_upstream(dir, upstream)?;
    ensure_upstream_checkout(dir, &upstream)?;
    let (subscription, eur_per_million_tokens) =
        select_cost_model(subscription, eur_per_million_tokens)?;
    let config_path = dir.join("mdboard.toml");
    if config_path.exists() && !force {
        bail!(
            "{} already exists; pass --force to overwrite the configuration",
            config_path.display()
        );
    }
    for path in [
        ".mdboard/cards",
        ".mdboard/events",
        ".mdboard/meetings",
        ".mdboard/reviews",
        ".mdboard/reports",
    ] {
        std::fs::create_dir_all(dir.join(path))?;
    }
    validate_upstream_branch(&upstream)?;
    let subscription = subscription.replace('\\', "\\\\").replace('"', "\\\"");
    let (guardian_ssh_key, guardian_ssh_fingerprint) = match ssh_key {
        Some(path) => {
            let public_key = std::fs::read_to_string(path)
                .with_context(|| format!("cannot read {}", path.display()))?
                .trim()
                .to_owned();
            let output = std::process::Command::new("ssh-keygen")
                .args(["-lf"])
                .arg(path)
                .output()
                .context("cannot run ssh-keygen")?;
            if !output.status.success() {
                bail!(
                    "invalid SSH public key: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            let fingerprint = String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .nth(1)
                .ok_or_else(|| anyhow::anyhow!("cannot read SSH fingerprint"))?
                .to_owned();
            (Some(public_key), Some(fingerprint))
        }
        None => (None, None),
    };
    let guardian_ssh = match (guardian_ssh_key, guardian_ssh_fingerprint) {
        (Some(key), Some(fingerprint)) => {
            format!("ssh_key = {key:?}\nssh_fingerprint = {fingerprint:?}\n")
        }
        _ => String::new(),
    };
    let config = format!(
        "mdboard = {version:?}\nschema = 3\n\n\
         [board]\nroot = \".mdboard\"\ncards = \"cards\"\nevents = \"events\"\n\
         meetings = \"meetings\"\nreviews = \"reviews\"\nreports = \"reports\"\nvotes = \"votes\"\norders = \"orders\"\n\
         attachments = \"attachments\"\n\
         upstream = \"upstream.md\"\nroadmap_override = \"roadmap-override.md\"\n\n\
         [stakeholders]\nmax_seats = 5\nquorum_seats = 3\nseat_term_days = 365\nseat_warning_days = 90\n\n\
         [guardian]\nname = \"Owner\"\nemail = \"owner@example.com\"\norganisation = \"\"\n{guardian_ssh}\n\
         [git]\nremote = \"origin\"\nstable = \"stable\"\nnightly = \"nightly\"\nupstream = {upstream:?}\n\n\
         [limits]\nroadmap = {roadmap_limit}\n\n\
         [approval]\nmode = \"chat\"\n\n\
         [meetings]\nconsortium = \"weekly\"\nrelease = \"biweekly\"\n\n\
         [costs]\nsubscription = {subscription:?}\neur_per_million_tokens = {rate}\n",
        version = generated_version::VERSION,
        upstream = upstream,
        subscription = subscription,
        rate = eur_per_million_tokens,
        roadmap_limit = roadmap_limit,
    );
    std::fs::write(&config_path, config)?;
    project_version::write_mdboard(dir)?;
    std::fs::write(
        dir.join(".mdboard/actors.toml"),
        format!(
            "[actors.owner]\nkind = \"human\"\ndisplay_name = \"Owner\"\nroles = [\"owner\"]\n\n\
         [actors.member]\nkind = \"human\"\ndisplay_name = \"Product member\"\nroles = [\"member\"]\n\n\
         [actors.agent]\nkind = \"agent\"\ndisplay_name = \"Coding agent\"\nroles = [\"agent\"]\ngit_name = \"{agent_name}\"\ngit_email = \"{agent_email}\"\n",
            agent_name = agent_git_name.as_deref().unwrap_or("agent"),
            agent_email = agent_git_email
                .as_deref()
                .unwrap_or("no-replay@example.com")
        ),
    )?;
    let ignore_path = dir.join(".mdboardignore");
    if !ignore_path.exists() {
        std::fs::write(
            &ignore_path,
            "# Paths never copied to a public mirror.\n.mdboard/\n.mdboardignore\n.env\n.env.*\n*.pem\n*.key\n*.p12\n.DS_Store\n.idea/\n.vscode/\n",
        )?;
    }
    let sample_path = dir.join(".mdboard/cards/0001.md");
    if !sample_path.exists() {
        let mut sample = ProcessCard::new_idea(
            "#0001",
            "First idea",
            "owner",
            mdboard_driver::process::now(),
        );
        sample.body =
            "## Intent\n\nDescribe the change you want the agent to shape and build.\n".to_owned();
        sample.validate()?;
        std::fs::write(&sample_path, sample.to_markdown()?)?;
    }
    if let Ok(loaded) = ProcessLoaded::load(dir) {
        for (path, content) in process_views(&loaded) {
            std::fs::write(&path, content)?;
        }
    }
    let agent_note = ensure_agent_guidance(dir)?;
    println!("mdboard init: wrote {}", config_path.display());
    println!("mdboard init: upstream branch {upstream}");
    println!(
        "mdboard init: cost model {subscription} at €{eur_per_million_tokens:.2} / 1M blended tokens"
    );
    if agent_note {
        println!("mdboard init: wrote AGENTS.md");
    }
    Ok(())
}

const AGENT_GUIDANCE_BEGIN: &str = "<!-- mdboard:agent-guidance:begin -->";
const AGENT_GUIDANCE_END: &str = "<!-- mdboard:agent-guidance:end -->";

const AGENT_GUIDANCE: &str = r#"<!-- mdboard:agent-guidance:begin -->
## This project is managed with mdBoard

This repository uses **mdBoard** for human intent, agent execution, review and Git-backed
decisions. `mdboard` must be installed and on `PATH`. If it is not, stop and ask the human owner
to install it; do not manage the board by editing its files by hand.

You are the **agent**. Humans own intent and decisions; you operate mdBoard and implement the
work. The humans talk to you; mdBoard coordinates both of you.

Begin every session with:

```sh
mdboard runbook
mdboard context
```

The runbook is normative: it defines your authority, the state machines, review, meetings and
branch rules. `context` is the current board. `actions <card>` tells you what is legal before you
act.

Rules:

1. Work only on the open `upstream` iteration. Do not create feature branches or worktrees.
   One agent writer holds the upstream lease.
2. Use named mdBoard actions. Never edit cards, events, reports or generated views by hand.
3. Act autonomously only where the runbook allows it: refine content in `refine`, estimate, break
   down, work tasks and hand over a batch. Create a spark only on explicit human instruction.
4. For every human-gated action, restate the effect, get an explicit human decision, and record
   the authoriser and the statement. Human gates include accepting/rejecting an idea, re-estimating
   a batch, putting it on the roadmap, starting implementation, accepting a batch review, merging
   and releasing. Record only what the human actually said; never invent consent.
5. Never merge `upstream -> nightly` or `nightly -> stable`; never release. The owner executes those
   operations. Never self-approve or accept your own work.
6. Review is per batch at `handover`, not per task. A reviewer must differ from the intent owner.
   Changes return the batch to `doing` and create rework tasks.
7. Respect one active batch in `breakdown`/`doing` and one task `working`. Finish, block or cancel
   before starting another.
8. Surface pending meeting gates in every brief. Never silently drop a meeting.
9. Refuse illegal requests and offer the legal alternative. Never delete a card, bypass a review or
   edit a generated view.
9a. Publication privacy: new cards default to public. When creating commercial, legal or partner
   cards, state that they can be private and how to mark them private. Never publish private card
   details, reports or attachments. Public issues/PRs may reference public cards only. Until the
   privacy/changelog cards are implemented, ask the owner before publishing.
10. After every state change, run `mdboard brief` and tell the human what changed, what needs them
    and what happens next.
11. Commit at task and batch boundaries with `mdboard commit -m "…"`; leave the working tree clean
    before the owner merges or releases.

Generated views are `BOARD.md` and `ROADMAP.md`; never hand-edit them.

Useful commands:

```sh
mdboard runbook
mdboard context
mdboard actions '#0007'
mdboard brief
mdboard history '#0007'
mdboard commit -m "message"
mdboard upstream status
```

Documentation: https://mdboard.io/guide/
Runbook guide: https://mdboard.io/guide/runbook
Repository: https://github.com/mdboard-io/mdboard
<!-- mdboard:agent-guidance:end -->"#;

fn ensure_git_repository(dir: &Path, init_git: bool) -> Result<()> {
    if std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_err()
    {
        bail!("git is required by mdboard but is not available on PATH");
    }
    let inside = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .output()
        .ok()
        .is_some_and(|output| output.status.success());
    if inside {
        return Ok(());
    }
    let initialize = init_git || {
        if !std::io::stdin().is_terminal() {
            bail!(
                "{} is not a Git repository; run `git init` or pass --init-git",
                dir.display()
            );
        }
        eprint!(
            "Initialize an empty Git repository in {}? [y/N]: ",
            dir.display()
        );
        use std::io::Write;
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    };
    if !initialize {
        bail!("mdboard requires a Git repository; initialization aborted");
    }
    let status = std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dir)
        .status()
        .context("cannot run git init")?;
    if !status.success() {
        let fallback = std::process::Command::new("git")
            .arg("init")
            .current_dir(dir)
            .status()
            .context("cannot run git init")?;
        if !fallback.success() {
            bail!("git init failed");
        }
    }
    Ok(())
}

fn select_upstream(dir: &Path, explicit: Option<&str>) -> Result<String> {
    let branches = mdboard_git::branches(dir).unwrap_or_default();
    let current = mdboard_git::current_branch(dir).ok();
    let preference = ["nightly", "develop", "development", "main", "master"];
    let default = preference
        .iter()
        .find(|candidate| branches.iter().any(|branch| branch == *candidate))
        .map(|candidate| (*candidate).to_owned())
        .or_else(|| branches.first().cloned())
        .or_else(|| current.clone())
        .unwrap_or_else(|| "main".to_owned());

    if let Some(branch) = explicit {
        let branch = branch.trim();
        validate_upstream_branch(branch)?;
        if !branches.is_empty() && !branches.iter().any(|candidate| candidate == branch) {
            bail!("upstream branch `{branch}` does not exist");
        }
        return Ok(branch.to_owned());
    }

    if !std::io::stdin().is_terminal() {
        return Ok(default);
    }
    use std::io::Write;
    if branches.is_empty() {
        eprint!("Upstream branch [{default}]: ");
    } else {
        eprint!(
            "Upstream branch [{}] (available: {}): ",
            default,
            branches.join(", ")
        );
    }
    std::io::stderr().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let selected = input.trim();
    let selected = if selected.is_empty() {
        default
    } else {
        selected.to_owned()
    };
    validate_upstream_branch(&selected)?;
    if !branches.is_empty() && !branches.iter().any(|branch| branch == &selected) {
        bail!("upstream branch `{selected}` does not exist");
    }
    Ok(selected)
}

fn validate_upstream_branch(branch: &str) -> Result<()> {
    let invalid = branch.is_empty()
        || branch.starts_with('-')
        || branch.contains("..")
        || branch.contains(['"', '\\'])
        || branch.chars().any(char::is_whitespace);
    if invalid {
        bail!("invalid upstream branch `{branch}`");
    }
    Ok(())
}

fn select_cost_model(
    subscription: Option<&str>,
    eur_per_million_tokens: Option<f64>,
) -> Result<(String, f64)> {
    let default_subscription = mdboard_core::config::DEFAULT_SUBSCRIPTION;
    let default_cost = mdboard_core::config::DEFAULT_EUR_PER_MILLION_TOKENS;

    let mut subscription = match subscription {
        Some(value) => {
            let value = value.trim();
            if value.is_empty() {
                bail!("subscription must not be empty");
            }
            Some(value.to_owned())
        }
        None => None,
    };
    let mut cost = eur_per_million_tokens;
    if let Some(value) = cost
        && (!value.is_finite() || value < 0.0)
    {
        bail!("cost per million tokens must be finite and non-negative");
    }

    if std::io::stdin().is_terminal() {
        use std::io::Write;
        if subscription.is_none() {
            eprint!(
                "Subscription or payment model [{default_subscription}] (e.g. DeepSeek V4 Flash On-Demand): "
            );
            std::io::stderr().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let value = input.trim();
            if !value.is_empty() {
                subscription = Some(value.to_owned());
            }
        }
        if cost.is_none() {
            eprint!(
                "Blended cost in EUR per 1M tokens [{default_cost}] (input + output; e.g. DeepSeek V4 Flash 0.21, Claude Opus 5.5 6.19): "
            );
            std::io::stderr().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let value = input.trim();
            if !value.is_empty() {
                let parsed = value
                    .replace(',', ".")
                    .parse::<f64>()
                    .with_context(|| format!("invalid cost per million tokens `{value}`"))?;
                if !parsed.is_finite() || parsed < 0.0 {
                    bail!("cost per million tokens must be finite and non-negative");
                }
                cost = Some(parsed);
            }
        }
    }

    let subscription = subscription.unwrap_or_else(|| default_subscription.to_owned());
    let cost = cost.unwrap_or(default_cost);
    if subscription.trim().is_empty() {
        bail!("subscription must not be empty");
    }
    Ok((subscription, cost))
}

fn ensure_agent_guidance(dir: &Path) -> Result<bool> {
    let path = dir.join("AGENTS.md");
    if !path.exists() {
        std::fs::write(&path, format!("# Agent instructions\n\n{AGENT_GUIDANCE}\n"))
            .with_context(|| format!("cannot write {}", path.display()))?;
        return Ok(true);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    if let Some(begin) = text.find(AGENT_GUIDANCE_BEGIN) {
        let end = text
            .find(AGENT_GUIDANCE_END)
            .map(|end| end + AGENT_GUIDANCE_END.len())
            .with_context(|| {
                format!(
                    "{} has a begin marker without an end marker",
                    path.display()
                )
            })?;
        if &text[begin..end] == AGENT_GUIDANCE {
            return Ok(false);
        }
        let mut updated = String::with_capacity(text.len());
        updated.push_str(&text[..begin]);
        updated.push_str(AGENT_GUIDANCE);
        updated.push_str(&text[end..]);
        std::fs::write(&path, updated)
            .with_context(|| format!("cannot write {}", path.display()))?;
        return Ok(true);
    }
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .with_context(|| format!("cannot append to {}", path.display()))?;
    use std::io::Write;
    write!(file, "\n\n{AGENT_GUIDANCE}\n")
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(true)
}

mod generated_version {
    include!(concat!(env!("OUT_DIR"), "/version.rs"));
}

#[cfg(test)]
mod agent_guidance_tests {
    use super::{AGENT_GUIDANCE, ensure_agent_guidance};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn fresh_file_gets_current_guidance() {
        let dir = TempDir::new().unwrap();
        assert!(ensure_agent_guidance(dir.path()).unwrap());
        let text = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(text.starts_with("# Agent instructions"));
        assert!(text.contains("You are the **agent**"));
        assert!(text.contains("mdboard runbook"));
        assert!(text.contains("Review is per batch at `handover`"));
    }

    #[test]
    fn existing_marked_block_is_replaced_and_custom_text_kept() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("AGENTS.md");
        fs::write(
            &path,
            "# Agent instructions\n\n<!-- mdboard:agent-guidance:begin -->\nold\n<!-- mdboard:agent-guidance:end -->\n\n## Custom\n\nkeep me\n",
        )
        .unwrap();
        assert!(ensure_agent_guidance(dir.path()).unwrap());
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("old\n"));
        assert!(text.contains("You are the **agent**"));
        assert!(text.contains("keep me"));
    }

    #[test]
    fn current_block_is_idempotent() {
        let dir = TempDir::new().unwrap();
        assert!(ensure_agent_guidance(dir.path()).unwrap());
        assert!(!ensure_agent_guidance(dir.path()).unwrap());
    }

    #[test]
    fn custom_file_is_appended() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("AGENTS.md");
        fs::write(&path, "# My project\n\nCustom rule.\n").unwrap();
        assert!(ensure_agent_guidance(dir.path()).unwrap());
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# My project"));
        assert!(text.contains("Custom rule."));
        assert!(text.contains("mdboard:agent-guidance:begin"));
    }

    #[test]
    fn malformed_marker_is_an_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("AGENTS.md");
        fs::write(&path, "# X\n\n<!-- mdboard:agent-guidance:begin -->\nold\n").unwrap();
        assert!(ensure_agent_guidance(dir.path()).is_err());
    }

    #[test]
    fn guidance_has_no_removed_process_terms() {
        for term in [
            "draft",
            "refinement",
            "ready",
            "DEVLOG",
            "OWNER_REPORTS",
            "KAIZEN",
            "card claim",
            "space new",
            "space merge",
        ] {
            assert!(!AGENT_GUIDANCE.contains(term), "stale term: {term}");
        }
    }
}
