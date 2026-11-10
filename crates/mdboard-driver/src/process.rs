//! Loading and writing the 1.0 process board.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use mdboard_core::config::{CostConfig, Plugins};
use mdboard_core::process::Branches;
use mdboard_core::process::{
    ApprovalMode, ContextConfig, Event, Guardian, Meeting, MeetingCadence, Order, Policy,
    ProcessCard, QualityGateConfig, ReviewArtifact, RoadmapOverride, RotationsFile,
    StakeholderConfig, UpstreamRecord, Vote,
};
use serde::{Deserialize, Serialize};

/// Default process directory and collection names.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BoardPaths {
    pub root: String,
    pub cards: String,
    pub events: String,
    pub meetings: String,
    pub reviews: String,
    pub reports: String,
    pub votes: String,
    pub orders: String,
    pub attachments: String,
    pub upstream: String,
    pub roadmap_override: String,
}

impl Default for BoardPaths {
    fn default() -> Self {
        Self {
            root: ".mdboard".to_owned(),
            cards: "cards".to_owned(),
            events: "events".to_owned(),
            meetings: "meetings".to_owned(),
            reviews: "reviews".to_owned(),
            reports: "reports".to_owned(),
            votes: "votes".to_owned(),
            orders: "orders".to_owned(),
            attachments: "attachments".to_owned(),
            upstream: "upstream.md".to_owned(),
            roadmap_override: "roadmap-override.md".to_owned(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessConfigFile {
    #[serde(default)]
    mdboard: Option<String>,
    #[serde(default)]
    schema: u32,
    #[serde(default)]
    board: BoardPaths,
    #[serde(default)]
    git: Branches,
    #[serde(default)]
    limits: LimitsFile,
    #[serde(default)]
    approval: ApprovalFile,
    #[serde(default)]
    meetings: MeetingCadence,
    #[serde(default)]
    costs: CostConfig,
    #[serde(default)]
    stakeholders: StakeholderConfig,
    #[serde(default)]
    guardian: Guardian,
    #[serde(default)]
    quality_gate: QualityGateConfig,
    #[serde(default)]
    context: ContextConfig,
    #[serde(default)]
    plugins: Plugins,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LimitsFile {
    roadmap: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ApprovalFile {
    mode: ApprovalMode,
}

impl Default for ApprovalFile {
    fn default() -> Self {
        Self {
            mode: ApprovalMode::Chat,
        }
    }
}

impl Default for LimitsFile {
    fn default() -> Self {
        Self { roadmap: 5 }
    }
}

/// A fully loaded 1.0 board.
#[derive(Debug)]
pub struct ProcessLoaded {
    pub root: PathBuf,
    pub schema: u32,
    pub version: Option<String>,
    pub policy: Policy,
    pub paths: BoardPaths,
    pub cards: Vec<ProcessCard>,
    pub card_paths: BTreeMap<String, PathBuf>,
    pub events: Vec<Event>,
    pub meetings: Vec<Meeting>,
    pub reviews: Vec<ReviewArtifact>,
    pub votes: Vec<Vote>,
    pub orders: Vec<Order>,
    pub rotations: Option<RotationsFile>,
    pub upstream: Option<UpstreamRecord>,
    pub roadmap_override: Option<RoadmapOverride>,
    pub plugins: Plugins,
    config_path: PathBuf,
}

impl ProcessLoaded {
    pub fn load(root: &Path) -> Result<Self> {
        let config_path = root.join("mdboard.toml");
        let text = fs::read_to_string(&config_path)
            .with_context(|| format!("cannot read {}", config_path.display()))?;
        let file: ProcessConfigFile = toml::from_str(&text)
            .with_context(|| format!("cannot parse {}", config_path.display()))?;
        if file.schema != 3 {
            bail!(
                "{} uses schema {}; the governance model requires schema 3",
                config_path.display(),
                file.schema
            );
        }
        let policy = Policy {
            branches: file.git,
            roadmap_limit: file.limits.roadmap,
            approval: file.approval.mode,
            meetings: file.meetings,
            costs: file.costs,
            stakeholders: file.stakeholders,
            guardian: file.guardian,
            quality_gate: file.quality_gate,
            context: file.context,
        };
        policy.validate()?;

        let process_root = root.join(&file.board.root);
        let cards_dir = process_root.join(&file.board.cards);
        let mut cards = Vec::new();
        let mut card_paths = BTreeMap::new();
        if cards_dir.exists() {
            let mut entries = fs::read_dir(&cards_dir)
                .with_context(|| format!("cannot read {}", cards_dir.display()))?
                .collect::<std::io::Result<Vec<_>>>()?;
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                    continue;
                }
                let card = ProcessCard::load(&path)?;
                if card_paths.insert(card.id.clone(), path).is_some() {
                    bail!("duplicate card id `{}`", card.id);
                }
                cards.push(card);
            }
        }
        cards.sort_by(|a, b| a.id.cmp(&b.id));

        let events = load_dir::<Event>(&process_root.join(&file.board.events))?;
        let meetings = load_dir::<Meeting>(&process_root.join(&file.board.meetings))?;
        let reviews = load_dir::<ReviewArtifact>(&process_root.join(&file.board.reviews))?;
        let votes = load_dir::<Vote>(&process_root.join(&file.board.votes))?;
        let orders = load_orders(&process_root.join(&file.board.orders))?;
        let rotations = load_rotations(&process_root)?;
        let upstream = load_one::<UpstreamRecord>(&process_root.join(&file.board.upstream))?;
        let roadmap_override =
            load_one::<RoadmapOverride>(&process_root.join(&file.board.roadmap_override))?;

        if let Some(record) = &upstream {
            record.validate()?;
        }
        if let Some(record) = &roadmap_override {
            record.validate()?;
        }
        for event in &events {
            event.validate()?;
        }
        for meeting in &meetings {
            meeting.validate()?;
        }
        for review in &reviews {
            review.validate()?;
        }
        for vote in &votes {
            vote.validate()?;
        }
        let mut order_ids = BTreeSet::new();
        let card_ids: BTreeSet<&str> = cards.iter().map(|card| card.id.as_str()).collect();
        for order in &orders {
            order.validate()?;
            if !order_ids.insert(order.id.clone()) {
                bail!("duplicate order id `{}`", order.id);
            }
            for card in &order.scope.cards {
                if !card_ids.contains(card.as_str()) {
                    bail!("order `{}` references unknown card `{card}`", order.id);
                }
            }
        }

        Ok(Self {
            root: root.to_path_buf(),
            schema: file.schema,
            version: file.mdboard,
            policy,
            paths: file.board,
            cards,
            card_paths,
            events,
            meetings,
            reviews,
            votes,
            orders,
            rotations,
            upstream,
            roadmap_override,
            plugins: file.plugins,
            config_path,
        })
    }

    #[must_use]
    pub fn process_root(&self) -> PathBuf {
        self.root.join(&self.paths.root)
    }

    #[must_use]
    pub fn cards_dir(&self) -> PathBuf {
        self.process_root().join(&self.paths.cards)
    }

    #[must_use]
    pub fn attachments_root(&self) -> PathBuf {
        self.process_root().join(&self.paths.attachments)
    }

    /// Directory holding one card's attachments.
    ///
    /// Card ids are path segments after stripping the leading `#`; ids with path
    /// separators or parent components are rejected by [`Self::attachment_path`].
    #[must_use]
    pub fn attachments_dir(&self, id: &str) -> PathBuf {
        self.attachments_root().join(id.trim_start_matches('#'))
    }

    /// Resolve a card attachment name without allowing it to escape the card's
    /// attachment directory.
    pub fn attachment_path(&self, id: &str, name: &str) -> Result<PathBuf> {
        let slug = attachment_slug(id)?;
        if name.trim().is_empty() {
            bail!("attachment name must not be empty");
        }
        let relative = Path::new(name);
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            bail!("attachment path must be relative to the card's attachment directory");
        }
        Ok(self.attachments_root().join(slug).join(relative))
    }

    /// List files under one card's attachment directory, recursively.
    pub fn attachment_files(&self, id: &str) -> Result<Vec<PathBuf>> {
        let dir = self.attachments_dir(id);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut files = Vec::new();
        collect_files(&dir, &mut files)?;
        files.sort();
        Ok(files)
    }

    /// Repository-relative prefix for all attachment references.
    #[must_use]
    pub fn attachment_prefix(&self) -> String {
        format!(
            "{}/{}/",
            self.paths.root.trim_end_matches('/'),
            self.paths.attachments.trim_matches('/')
        )
    }

    #[must_use]
    pub fn find(&self, id: &str) -> Option<&ProcessCard> {
        self.cards.iter().find(|card| card.id == id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut ProcessCard> {
        self.cards.iter_mut().find(|card| card.id == id)
    }

    pub fn require(&self, id: &str) -> Result<&ProcessCard> {
        self.find(id)
            .with_context(|| format!("card `{id}` not found"))
    }

    pub fn require_mut(&mut self, id: &str) -> Result<&mut ProcessCard> {
        self.find_mut(id)
            .with_context(|| format!("card `{id}` not found"))
    }

    #[must_use]
    pub fn roadmap_count(&self) -> u32 {
        let count = self
            .cards
            .iter()
            .filter(|card| {
                card.kind == mdboard_core::process::Kind::Batch
                    && card.state == mdboard_core::process::State::Roadmap
            })
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    pub fn active_batches(&self) -> impl Iterator<Item = &ProcessCard> {
        self.cards.iter().filter(|card| {
            card.kind == mdboard_core::process::Kind::Batch && card.state.is_batch_active()
        })
    }

    pub fn working_tasks(&self) -> impl Iterator<Item = &ProcessCard> {
        self.cards.iter().filter(|card| {
            card.kind == mdboard_core::process::Kind::Task && card.state.is_task_active()
        })
    }

    #[must_use]
    pub fn effective_roadmap_limit(&self) -> Option<u32> {
        self.roadmap_override.as_ref().and_then(|record| {
            let expires = chrono::DateTime::parse_from_rfc3339(&record.expires_at).ok()?;
            (expires > chrono::Utc::now()).then_some(record.limit)
        })
    }

    #[must_use]
    pub fn roadmap_limit(&self) -> u32 {
        self.effective_roadmap_limit()
            .unwrap_or(self.policy.roadmap_limit)
    }

    #[must_use]
    pub fn estimated_cost_eur(&self) -> f64 {
        let weighted: f64 = self
            .cards
            .iter()
            .filter(|card| card.state == mdboard_core::process::State::Roadmap)
            .filter_map(|card| card.estimate.as_ref())
            .map(|estimate| estimate.tokens.weighted)
            .sum();
        let cost = weighted / 1_000_000.0 * self.policy.costs.eur_per_million_tokens;
        if cost == 0.0 { 0.0 } else { cost }
    }

    pub fn save_card(&mut self, card: &mut ProcessCard) -> Result<()> {
        card.updated_at = now();
        card.validate()?;
        let path = self
            .card_paths
            .get(&card.id)
            .cloned()
            .unwrap_or_else(|| self.cards_dir().join(card_file_name(&card.id)));
        write_text(&path, &card.to_markdown()?)?;
        if !self.card_paths.contains_key(&card.id) {
            self.card_paths.insert(card.id.clone(), path);
            self.cards.push(card.clone());
            self.cards.sort_by(|a, b| a.id.cmp(&b.id));
        } else if let Some(existing) = self.find_mut(&card.id) {
            *existing = card.clone();
        }
        Ok(())
    }

    pub fn create_card(&mut self, card: ProcessCard) -> Result<()> {
        if self.card_paths.contains_key(&card.id) {
            bail!("card `{}` already exists", card.id);
        }
        let path = self.cards_dir().join(card_file_name(&card.id));
        write_text(&path, &card.to_markdown()?)?;
        self.card_paths.insert(card.id.clone(), path);
        self.cards.push(card);
        self.cards.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(())
    }

    pub fn write_event(&mut self, event: &Event) -> Result<()> {
        event.validate()?;
        let path = self
            .process_root()
            .join(&self.paths.events)
            .join(format!("{}.md", event.id));
        write_text(&path, &yaml_markdown(event)?)?;
        self.events.push(event.clone());
        Ok(())
    }

    pub fn write_meeting(&mut self, meeting: &Meeting) -> Result<()> {
        meeting.validate()?;
        let path = self
            .process_root()
            .join(&self.paths.meetings)
            .join(format!("{}.md", meeting.id));
        write_text(&path, &yaml_markdown(meeting)?)?;
        if let Some(existing) = self.meetings.iter_mut().find(|item| item.id == meeting.id) {
            *existing = meeting.clone();
        } else {
            self.meetings.push(meeting.clone());
        }
        Ok(())
    }

    pub fn write_review(&mut self, review: &ReviewArtifact) -> Result<()> {
        review.validate()?;
        let path = self
            .process_root()
            .join(&self.paths.reviews)
            .join(format!("{}.md", review.id));
        write_text(&path, &yaml_markdown(review)?)?;
        self.reviews.push(review.clone());
        Ok(())
    }

    pub fn write_upstream(&mut self, record: &UpstreamRecord) -> Result<()> {
        record.validate()?;
        let path = self.process_root().join(&self.paths.upstream);
        write_text(&path, &yaml_markdown(record)?)?;
        self.upstream = Some(record.clone());
        Ok(())
    }

    pub fn write_roadmap_override(&mut self, record: Option<&RoadmapOverride>) -> Result<()> {
        let path = self.process_root().join(&self.paths.roadmap_override);
        if let Some(record) = record {
            record.validate()?;
            write_text(&path, &yaml_markdown(record)?)?;
        } else if path.exists() {
            fs::remove_file(&path).with_context(|| format!("cannot remove {}", path.display()))?;
        }
        self.roadmap_override = record.cloned();
        Ok(())
    }

    pub fn write_report(&self, batch: &ProcessCard, body: &str) -> Result<PathBuf> {
        let path = self
            .process_root()
            .join(&self.paths.reports)
            .join(format!("R{}.md", batch.id.trim_start_matches('#')));
        write_text(&path, body)?;
        Ok(path)
    }

    pub fn append_changelog(&self, batch: &ProcessCard, report: &Path) -> Result<()> {
        let path = self.root.join("CHANGELOG.md");
        let mut text = if path.exists() {
            fs::read_to_string(&path)?
        } else {
            "# Changelog\n\n".to_owned()
        };
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let heading = format!("## {date}");
        if !text.contains(&heading) {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&format!("\n{heading}\n\n"));
        }
        let title = batch.title.replace('\n', " ");
        let report = report
            .strip_prefix(&self.root)
            .unwrap_or(report)
            .display()
            .to_string();
        text.push_str(&format!("- {title} ([{} report]({report}))\n", batch.id));
        write_text(&path, &text)
    }

    pub fn next_idea_or_batch_id(&self) -> String {
        let max = self
            .cards
            .iter()
            .filter(|card| card.kind != mdboard_core::process::Kind::Task)
            .filter_map(|card| card.id.trim_start_matches('#').parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        format!("#{:04}", max + 1)
    }

    pub fn next_task_id(&self, parent: &str) -> String {
        let prefix = format!("{parent}-");
        let max = self
            .cards
            .iter()
            .filter_map(|card| card.id.strip_prefix(&prefix))
            .filter_map(|suffix| suffix.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        format!("{parent}-{:02}", max + 1)
    }

    pub fn next_meeting_id(&self, batch: &str) -> String {
        let prefix = format!("M{}-", batch.trim_start_matches('#'));
        let max = self
            .meetings
            .iter()
            .filter_map(|meeting| meeting.id.strip_prefix(&prefix))
            .filter_map(|suffix| suffix.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        format!("{prefix}{:02}", max + 1)
    }

    pub fn next_review_id(&self, batch: &str, round: u32) -> String {
        let prefix = format!("RV-{}-{round:02}-", batch.trim_start_matches('#'));
        let max = self
            .reviews
            .iter()
            .filter_map(|review| review.id.strip_prefix(&prefix))
            .filter_map(|suffix| suffix.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        format!("{prefix}{:02}", max + 1)
    }

    pub fn next_event_id(&self, action: &str, card: Option<&str>) -> String {
        let stamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
        let card = card
            .map(|id| id.trim_start_matches('#').to_owned())
            .unwrap_or_else(|| "board".to_owned());
        let mut candidate = format!("E-{stamp}-{card}-{action}");
        let mut suffix = 1_u32;
        while self.events.iter().any(|event| event.id == candidate) {
            candidate = format!("E-{stamp}-{card}-{action}-{suffix}");
            suffix += 1;
        }
        candidate
    }

    #[must_use]
    pub fn history(&self, id: &str) -> Vec<&Event> {
        self.events
            .iter()
            .filter(|event| event.card.as_deref() == Some(id))
            .collect()
    }

    #[must_use]
    pub fn pending_meetings(&self) -> Vec<&Meeting> {
        self.meetings
            .iter()
            .filter(|meeting| {
                matches!(
                    meeting.status,
                    mdboard_core::process::MeetingStatus::Pending
                        | mdboard_core::process::MeetingStatus::Deferred
                )
            })
            .collect()
    }

    #[must_use]
    pub fn open_clarification_count(&self, batch: &str) -> usize {
        self.meetings
            .iter()
            .filter(|meeting| {
                meeting.kind == mdboard_core::process::MeetingKind::BreakdownClarification
                    && meeting.linked_cards.iter().any(|id| id == batch)
                    && matches!(
                        meeting.status,
                        mdboard_core::process::MeetingStatus::Pending
                            | mdboard_core::process::MeetingStatus::Deferred
                    )
                    && meeting.questions.iter().any(|question| !question.resolved)
            })
            .count()
    }

    #[must_use]
    pub fn accepted_review_for(&self, batch: &str, intent_owner: &str) -> Option<&ReviewArtifact> {
        let round = self
            .reviews
            .iter()
            .filter(|review| review.batch == batch)
            .map(|review| review.round)
            .max()?;
        let latest = self
            .reviews
            .iter()
            .filter(|review| review.batch == batch && review.round == round)
            .collect::<Vec<_>>();
        if latest
            .iter()
            .any(|review| review.verdict == mdboard_core::process::Verdict::ChangesRequired)
        {
            return None;
        }
        latest.into_iter().find(|review| {
            review.verdict == mdboard_core::process::Verdict::Accepted
                && review.reviewer != intent_owner
        })
    }

    pub fn write_vote(&mut self, vote: &Vote) -> Result<()> {
        vote.validate()?;
        let path = self
            .process_root()
            .join(&self.paths.votes)
            .join(format!("{}.md", vote.id));
        write_text(&path, &yaml_markdown(vote)?)?;
        if let Some(existing) = self.votes.iter_mut().find(|item| item.id == vote.id) {
            *existing = vote.clone();
        } else {
            self.votes.push(vote.clone());
        }
        Ok(())
    }

    #[must_use]
    pub fn latest_vote_for(&self, card: &str) -> Option<&Vote> {
        self.votes
            .iter()
            .filter(|vote| vote.card == card)
            .max_by(|a, b| a.opened_at.cmp(&b.opened_at))
    }

    #[must_use]
    pub fn next_vote_id(&self, card: &str) -> String {
        format!(
            "V-{}-{:02}",
            card.trim_start_matches('#'),
            self.votes.iter().filter(|vote| vote.card == card).count() + 1
        )
    }

    pub fn write_order(&mut self, order: &Order) -> Result<()> {
        order.validate()?;
        let path = self
            .process_root()
            .join(&self.paths.orders)
            .join(format!("{}.md", order.id));
        write_text(&path, &order.to_markdown()?)?;
        if let Some(existing) = self.orders.iter_mut().find(|item| item.id == order.id) {
            *existing = order.clone();
        } else {
            self.orders.push(order.clone());
        }
        Ok(())
    }

    #[must_use]
    pub fn next_order_id(&self) -> String {
        let max = self
            .orders
            .iter()
            .filter_map(|order| order.id.strip_prefix("SO-"))
            .filter_map(|value| value.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        format!("SO-{:04}", max + 1)
    }

    #[must_use]
    pub fn active_orders(&self) -> Vec<&Order> {
        self.orders
            .iter()
            .filter(|order| order.status == mdboard_core::process::OrderStatus::Active)
            .collect()
    }

    #[must_use]
    pub fn applicable_orders(&self, card: &ProcessCard) -> Vec<&Order> {
        let today = chrono::Utc::now().date_naive().to_string();
        self.orders
            .iter()
            .filter(|order| order.applies_to(card, &today))
            .collect()
    }

    #[must_use]
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    #[must_use]
    pub fn card_paths(&self) -> &BTreeMap<String, PathBuf> {
        &self.card_paths
    }

    #[must_use]
    pub fn all_ids(&self) -> BTreeSet<String> {
        self.cards.iter().map(|card| card.id.clone()).collect()
    }
}

fn load_rotations(process_root: &Path) -> Result<Option<RotationsFile>> {
    let path = process_root.join("rotations.toml");
    if !path.exists() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let file: RotationsFile =
        toml::from_str(&text).with_context(|| format!("cannot parse {}", path.display()))?;
    if file.schema != 1 {
        bail!("rotations.toml uses unsupported schema {}", file.schema);
    }
    for (key, rotation) in &file.rotations {
        if rotation.title.trim().is_empty()
            || rotation.description.trim().is_empty()
            || rotation.current.trim().is_empty()
        {
            bail!("rotation `{key}` is missing title, description or current holder");
        }
        if rotation.auto_rotate_days == Some(0) {
            bail!("rotation `{key}` auto_rotate_days must be greater than zero");
        }
        let mut effective: Vec<String> = file
            .people
            .values()
            .map(|person| person.email.to_lowercase())
            .filter(|email| {
                !rotation
                    .remove
                    .iter()
                    .any(|removed| removed.eq_ignore_ascii_case(email))
            })
            .collect();
        effective.extend(
            rotation
                .add
                .iter()
                .map(|person| person.email.to_lowercase()),
        );
        if effective.iter().any(|email| email.trim().is_empty()) {
            bail!("rotation `{key}` has an empty participant e-mail");
        }
        let unique: BTreeSet<_> = effective.iter().collect();
        if unique.len() != effective.len() {
            bail!("rotation `{key}` has duplicate participants");
        }
        if !effective
            .iter()
            .any(|email| email == &rotation.current.to_lowercase())
        {
            bail!("rotation `{key}` current holder is not an effective participant");
        }
        for removed in &rotation.remove {
            if rotation
                .add
                .iter()
                .any(|person| person.email.eq_ignore_ascii_case(removed))
            {
                bail!("rotation `{key}` removes and adds `{removed}`");
            }
        }
    }
    Ok(Some(file))
}

fn load_orders(dir: &Path) -> Result<Vec<Order>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut orders = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }
        let text =
            fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
        orders.push(Order::parse(&path, &text)?);
    }
    orders.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(orders)
}

fn load_dir<T>(dir: &Path) -> Result<Vec<T>>
where
    T: for<'de> Deserialize<'de> + Validate,
{
    let mut items = Vec::new();
    if !dir.exists() {
        return Ok(items);
    }
    let mut entries = fs::read_dir(dir)
        .with_context(|| format!("cannot read {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let item = load_yaml_markdown::<T>(&path)?;
        items.push(item);
    }
    Ok(items)
}

fn load_one<T>(path: &Path) -> Result<Option<T>>
where
    T: for<'de> Deserialize<'de> + Validate,
{
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(load_yaml_markdown::<T>(path)?))
}

trait Validate {
    fn validate(&self) -> mdboard_core::Result<()>;
}

impl Validate for Event {
    fn validate(&self) -> mdboard_core::Result<()> {
        Event::validate(self)
    }
}

impl Validate for Meeting {
    fn validate(&self) -> mdboard_core::Result<()> {
        Meeting::validate(self)
    }
}

impl Validate for ReviewArtifact {
    fn validate(&self) -> mdboard_core::Result<()> {
        ReviewArtifact::validate(self)
    }
}

impl Validate for UpstreamRecord {
    fn validate(&self) -> mdboard_core::Result<()> {
        UpstreamRecord::validate(self)
    }
}

impl Validate for Vote {
    fn validate(&self) -> mdboard_core::Result<()> {
        Vote::validate(self)
    }
}

impl Validate for RoadmapOverride {
    fn validate(&self) -> mdboard_core::Result<()> {
        RoadmapOverride::validate(self)
    }
}

fn load_yaml_markdown<T>(path: &Path) -> Result<T>
where
    T: for<'de> Deserialize<'de> + Validate,
{
    let text =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let (yaml, _) = split_front_matter(&text)
        .with_context(|| format!("missing YAML front matter in {}", path.display()))?;
    let value: T =
        serde_yaml::from_str(&yaml).with_context(|| format!("cannot parse {}", path.display()))?;
    value.validate()?;
    Ok(value)
}

fn yaml_markdown<T: Serialize>(value: &T) -> Result<String> {
    let yaml = serde_yaml::to_string(value)?;
    Ok(format!("---\n{yaml}---\n"))
}

fn split_front_matter(text: &str) -> Option<(String, String)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return None;
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
    in_body.then_some((yaml, body))
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).with_context(|| format!("cannot write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}

fn card_file_name(id: &str) -> String {
    format!("{}.md", id.trim_start_matches('#'))
}

fn attachment_slug(id: &str) -> Result<&str> {
    let slug = id.trim_start_matches('#');
    if slug.is_empty()
        || slug == "."
        || slug == ".."
        || slug
            .chars()
            .any(|ch| ch == '/' || ch == '\\' || ch.is_control())
    {
        bail!("invalid card id `{id}` for attachments");
    }
    Ok(slug)
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files(&path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

#[must_use]
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
