//! Process-board HTML and JSON API for the 1.0 model.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::{Path as AxumPath, State as AxumState};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use mdboard_actions::process;
use mdboard_core::process::{Action, ProcessCard, State, Verdict};
use mdboard_driver::process::ProcessLoaded;
const STYLE: &str = include_str!("../assets/style.css");
const LOGO: &str = include_str!("../assets/logo.svg");
use notify::RecursiveMode;
use notify_debouncer_mini::{DebounceEventResult, new_debouncer};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tower_http::services::ServeDir;

use crate::{ApiError, AppState, BuildOutput, require_write};

const SCRIPT: &str = r#"
async function csrf() {
  const response = await fetch('/api/auth/me');
  if (!response.ok) { window.location = '/login.html'; throw new Error('signed out'); }
  return (await response.json()).csrf;
}
async function post(path, body) {
  const token = await csrf();
  const response = await fetch(path, {
    method: 'POST',
    headers: {'content-type': 'application/json', 'x-csrf-token': token},
    body: JSON.stringify(body || {})
  });
  if (!response.ok) {
    const value = await response.json().catch(() => ({error: response.statusText}));
    alert(value.error || 'request failed');
    return false;
  }
  location.reload();
  return true;
}
function reviewBatch(id) {
  const findings = document.getElementById('findings').value.split('\n').filter(Boolean).map((text, index) => ({id: 'F' + (index + 1), text}));
  post('/api/batches/' + encodeURIComponent(id) + '/review', {
    reviewer: document.getElementById('reviewer').value,
    authored_by: document.getElementById('authored_by').value,
    verdict: document.getElementById('verdict').value,
    findings,
    round: Number(document.getElementById('round').value || 1)
  });
}
function acceptIdea(id) { post('/api/ideas/' + encodeURIComponent(id) + '/accept', {}); }
function rejectIdea(id) {
  const reason = prompt('Rejection reason?');
  if (reason) post('/api/ideas/' + encodeURIComponent(id) + '/reject', {reason});
}
function completeBatch(id) { post('/api/batches/' + encodeURIComponent(id) + '/complete', {}); }
function requestChanges(id) {
  const reason = prompt('Reason for changes?');
  if (reason) post('/api/batches/' + encodeURIComponent(id) + '/request-changes', {reason});
}
function meetingAction(id, action) {
  if (action === 'defer') {
    const until = prompt('Defer until (RFC 3339)?');
    if (until) post('/api/meetings/' + encodeURIComponent(id) + '/defer', {until});
  } else if (action === 'dismiss') {
    const reason = prompt('Dismissal reason?');
    if (reason) post('/api/meetings/' + encodeURIComponent(id) + '/dismiss', {reason});
  } else {
    post('/api/meetings/' + encodeURIComponent(id) + '/accept', {});
  }
}
async function login(event) {
  event.preventDefault();
  const token = document.getElementById('token').value;
  const response = await fetch('/api/auth/login', {method: 'POST', headers: {'content-type': 'application/json'}, body: JSON.stringify({token})});
  if (!response.ok) { alert('invalid token'); return; }
  location = '/';
}
function applyFilter() {
  const value = location.hash.startsWith('#state=') ? location.hash.slice(7) : 'all';
  document.querySelectorAll('.kcard').forEach((card) => {
    const visible = value === 'all'
      || (value === 'open' ? card.dataset.gate === '1' : card.dataset.state === value);
    card.hidden = !visible;
  });
  const current = location.pathname + location.hash;
  document.querySelectorAll('.panel-item').forEach((item) => {
    item.classList.toggle('is-active', value !== 'all' && item.getAttribute('href') === current);
  });
  if (value === 'all') {
    document.querySelector('.panel-item[href="/board.html"]')?.classList.add('is-active');
  }
}

async function initShell() {
  try {
    const response = await fetch('/api/auth/me');
    if (response.ok) {
      const me = await response.json();
      document.body.dataset.auth = 'user';
      const name = document.querySelector('.signed-name');
      const avatar = document.querySelector('.signed-in .avatar');
      if (name) name.textContent = me.actor;
      if (avatar) avatar.textContent = String(me.actor).slice(0, 2).toUpperCase();
    }
  } catch (_) {}
  const saved = localStorage.getItem('mdboard-theme');
  const dark = saved ? saved === 'dark' : window.matchMedia('(prefers-color-scheme: dark)').matches;
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  document.querySelector('.theme-toggle')?.addEventListener('click', () => {
    const next = document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark';
    document.documentElement.dataset.theme = next;
    localStorage.setItem('mdboard-theme', next);
  });
  applyFilter();
  window.addEventListener('hashchange', applyFilter);
}

initShell();

try {
  const protocol = location.protocol === 'https:' ? 'wss://' : 'ws://';
  const socket = new WebSocket(protocol + location.host + '/ws');
  socket.onmessage = () => location.reload();
} catch (_) {}
"#;

pub fn build(root: &Path) -> Result<BuildOutput> {
    let loaded = ProcessLoaded::load(root)?;
    let visible: Vec<_> = loaded
        .cards
        .iter()
        .filter(|card| card.archived_at.is_none())
        .cloned()
        .collect();
    let plan = mdboard_renderer::process::render_process_views(
        &visible,
        &loaded.policy,
        &loaded.meetings,
        loaded.upstream.as_ref(),
        &loaded.root,
        loaded.roadmap_limit(),
    );
    for (path, content) in plan.files {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)
            .with_context(|| format!("cannot write {}", path.display()))?;
    }

    let site_dir = loaded.process_root().join("site");
    if site_dir.exists() {
        std::fs::remove_dir_all(&site_dir)?;
    }
    for (relative, content) in site_files(&loaded) {
        let path = site_dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    Ok(BuildOutput {
        site_dir,
        cards: loaded.cards.len(),
        findings: 0,
        upstream: loaded.policy.branches.upstream,
    })
}

pub fn watch(root: &Path) -> Result<()> {
    let root = root.to_path_buf();
    let first = build(&root)?;
    crate::report(&first);
    let (_debouncer, receiver) = start_watcher(&root)?;
    for event in receiver {
        match event {
            Ok(()) => match build(&root) {
                Ok(output) => crate::report(&output),
                Err(error) => eprintln!("rebuild failed: {error:#}"),
            },
            Err(error) => eprintln!("watch error: {error}"),
        }
    }
    Ok(())
}

pub fn spawn_watcher(root: PathBuf, sender: broadcast::Sender<()>) -> Result<()> {
    let (_debouncer, receiver) = start_watcher(&root)?;
    std::thread::spawn(move || {
        for event in receiver {
            match event {
                Ok(()) => match build(&root) {
                    Ok(output) => {
                        crate::report(&output);
                        let _ = sender.send(());
                    }
                    Err(error) => eprintln!("rebuild failed: {error:#}"),
                },
                Err(error) => eprintln!("watch error: {error}"),
            }
        }
        drop(_debouncer);
    });
    Ok(())
}

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/ws", get(crate::websocket))
        .route("/api/auth/login", post(crate::login))
        .route("/api/auth/logout", post(crate::logout))
        .route("/api/auth/me", get(crate::me))
        .route("/api/board.json", get(board_json))
        .route("/api/cards/{id}", get(card_json))
        .route("/api/meetings.json", get(meetings_json))
        .route("/api/schema.json", get(schema_json))
        .route("/api/ideas/{id}/accept", post(accept_idea))
        .route("/api/ideas/{id}/reject", post(reject_idea))
        .route("/api/batches/{id}/review", post(review_batch))
        .route("/api/batches/{id}/complete", post(complete_batch))
        .route("/api/batches/{id}/request-changes", post(request_changes))
        .route("/api/meetings/{id}/accept", post(meeting_accept))
        .route("/api/meetings/{id}/defer", post(meeting_defer))
        .route("/api/meetings/{id}/dismiss", post(meeting_dismiss))
        .with_state(state.clone())
        .fallback_service(
            ServeDir::new(state.site_dir.clone()).append_index_html_on_directories(true),
        )
}

type Watch = (
    notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>,
    std::sync::mpsc::Receiver<Result<(), String>>,
);

fn start_watcher(root: &Path) -> Result<Watch> {
    let process = ProcessLoaded::load(root)?.process_root();
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(
        Duration::from_millis(250),
        move |result: DebounceEventResult| {
            let message = match result {
                Ok(events) => {
                    let relevant = events.iter().any(|event| {
                        let path = event.path.to_string_lossy().replace('\\', "/");
                        !path.contains("/site/")
                            && (path.ends_with(".md") || path.ends_with(".toml"))
                    });
                    if !relevant {
                        return;
                    }
                    Ok(())
                }
                Err(error) => Err(error.to_string()),
            };
            let _ = sender.send(message);
        },
    )
    .context("cannot start the filesystem watcher")?;
    debouncer
        .watcher()
        .watch(&process, RecursiveMode::Recursive)
        .with_context(|| format!("cannot watch {}", process.display()))?;
    debouncer
        .watcher()
        .watch(&root.join("mdboard.toml"), RecursiveMode::NonRecursive)
        .context("cannot watch mdboard.toml")?;
    Ok((debouncer, receiver))
}

fn site_files(loaded: &ProcessLoaded) -> BTreeMap<PathBuf, String> {
    let mut files = BTreeMap::new();
    files.insert(PathBuf::from("index.html"), index_page(loaded));
    files.insert(PathBuf::from("board.html"), board_page(loaded));
    files.insert(PathBuf::from("roadmap.html"), roadmap_page(loaded));
    files.insert(PathBuf::from("meetings.html"), meetings_page(loaded));
    files.insert(PathBuf::from("login.html"), login_page());
    files.insert(PathBuf::from("assets/style.css"), STYLE.to_owned());
    files.insert(PathBuf::from("assets/process.js"), SCRIPT.to_owned());
    files.insert(PathBuf::from("assets/logo.svg"), LOGO.to_owned());
    files.insert(
        PathBuf::from("api/board.json"),
        pretty(&board_value(loaded)),
    );
    files.insert(
        PathBuf::from("api/meetings.json"),
        pretty(&serde_json::to_value(&loaded.meetings).unwrap_or(Value::Null)),
    );
    files.insert(PathBuf::from("api/schema.json"), pretty(&schema_value()));
    for card in &loaded.cards {
        let slug = card.id.trim_start_matches('#');
        files.insert(
            PathBuf::from(format!("cards/{slug}.html")),
            card_page(loaded, card),
        );
        files.insert(
            PathBuf::from(format!("api/cards/{slug}.json")),
            pretty(&card_value(loaded, card)),
        );
    }
    files
}

const ICON_HOME: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><path d="M9 22V12h6v10"/></svg>"#;
const ICON_BOARD: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M8 7v10M16 7v10M3 12h18"/></svg>"#;
const ICON_MAP: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 6 3 3v15l6 3 6-3 6 3V6l-6 3-6-3z"/><path d="M9 6v15M15 9v15"/></svg>"#;
const ICON_USERS: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75"/></svg>"#;
const ICON_CHEVRON: &str =
    r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m15 18-6-6 6-6"/></svg>"#;

fn layout(title: &str, loaded: &ProcessLoaded, body: &str) -> String {
    let active = if title == "Roadmap" {
        "roadmap"
    } else if title == "Meetings" {
        "meetings"
    } else if title == "Cards" {
        "board"
    } else {
        "home"
    };
    let panel_title = match active {
        "roadmap" => "Roadmap",
        "meetings" => "Meetings",
        "board" => "Board",
        _ => "Overview",
    };
    let panel = generic_panel(loaded, active);
    layout_with_panel(title, loaded, active, panel_title, &panel, body)
}

fn layout_with_panel(
    title: &str,
    _loaded: &ProcessLoaded,
    active: &str,
    panel_title: &str,
    panel: &str,
    body: &str,
) -> String {
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title} · mdBoard</title><link rel="stylesheet" href="/assets/style.css"><script defer src="/assets/process.js"></script></head><body class="app" data-auth="anon"><nav class="rail"><a class="rail-logo" href="/" title="Overview"><img src="/assets/logo.svg" alt="mdBoard"></a><a class="rail-item {home}" href="/" title="Overview">{icon_home}</a><a class="rail-item {board}" href="/board.html" title="Board">{icon_board}</a><a class="rail-item {roadmap}" href="/roadmap.html" title="Roadmap">{icon_map}</a><a class="rail-item {meetings}" href="/meetings.html" title="Meetings">{icon_users}</a></nav><aside class="panel"><div class="panel-head"><span>{panel_title}</span><button class="panel-collapse" aria-label="Collapse panel">{icon_chevron}</button></div>{panel}</aside><div class="main"><header class="topbar"><button class="theme-toggle" aria-label="Toggle dark mode"></button><span class="topbar-sep"></span><span class="signed-in auth-only"><span class="avatar">OW</span><span class="signed-name">owner</span></span><a class="btn btn-secondary signed-out anon-only" href="/login.html">Sign in</a></header><main class="content">{body}<p class="content-foot">Generated from the open upstream iteration. The HTML board is a view; Git is truth.</p></main></div></body></html>"#,
        title = escape(title),
        home = if active == "home" { "is-active" } else { "" },
        board = if active == "board" { "is-active" } else { "" },
        roadmap = if active == "roadmap" { "is-active" } else { "" },
        meetings = if active == "meetings" {
            "is-active"
        } else {
            ""
        },
        panel_title = escape(panel_title),
        panel = panel,
        icon_home = ICON_HOME,
        icon_board = ICON_BOARD,
        icon_map = ICON_MAP,
        icon_users = ICON_USERS,
        icon_chevron = ICON_CHEVRON,
        body = body
    )
}

fn generic_panel(loaded: &ProcessLoaded, active: &str) -> String {
    format!(
        r#"<a class="panel-item {}" href="/board.html">All cards <span class="count">{}</span></a><a class="panel-item {}" href="/roadmap.html">Roadmap <span class="count">{}</span></a><a class="panel-item {}" href="/meetings.html">Meetings <span class="count">{}</span></a><p class="panel-note">The open upstream iteration is the working truth. Views are generated.</p>"#,
        if active == "board" { "is-active" } else { "" },
        loaded.cards.len(),
        if active == "roadmap" { "is-active" } else { "" },
        loaded.roadmap_count(),
        if active == "meetings" {
            "is-active"
        } else {
            ""
        },
        loaded.pending_meetings().len()
    )
}

fn index_page(loaded: &ProcessLoaded) -> String {
    let mut body = String::new();
    body.push_str("<div class=\"page-head\"><p class=\"eyebrow\">Human review</p><h1>Board overview</h1></div>");
    body.push_str(&format!(
        "<div class=\"stat-grid\"><a class=\"stat\" href=\"/roadmap.html\"><span class=\"stat-value\">{}</span><span class=\"stat-label\">roadmap / {}</span></a><div class=\"stat\"><span class=\"stat-value\">{}</span><span class=\"stat-label\">cards</span></div><div class=\"stat\"><span class=\"stat-value\">{}</span><span class=\"stat-label\">pending meetings</span></div><div class=\"stat\"><span class=\"stat-value\">€{:.4}</span><span class=\"stat-label\">estimated cost</span></div></div>",
        loaded.roadmap_count(), loaded.roadmap_limit(), loaded.cards.len(), loaded.pending_meetings().len(), loaded.estimated_cost_eur()
    ));
    body.push_str("<h2>Needs you</h2>");
    let pending = loaded.pending_meetings();
    if pending.is_empty() {
        body.push_str("<p class=\"muted\">Nothing pending.</p>");
    } else {
        body.push_str("<ul class=\"link-list\">");
        for meeting in pending {
            body.push_str(&format!(
                "<li><a href=\"/meetings.html\">{}</a> — {:?}</li>",
                escape(&meeting.id),
                meeting.kind
            ));
        }
        body.push_str("</ul>");
    }
    body.push_str("<h2>Active work</h2>");
    let active = loaded.active_batches().collect::<Vec<_>>();
    if active.is_empty() {
        body.push_str("<p class=\"muted\">No batch in breakdown, doing or handover.</p>");
    } else {
        body.push_str("<div class=\"card-grid\">");
        for card in active {
            body.push_str(&card_tile(card));
        }
        body.push_str("</div>");
    }
    layout("Overview", loaded, &body)
}

const PANEL_GROUPS: &[(&str, &[State])] = &[
    ("Intent", &[State::Draft, State::Refine, State::Pitch]),
    ("Plan", &[State::Accepted, State::Roadmap]),
    ("Execute", &[State::Breakdown, State::Doing, State::Blocked]),
    ("Review", &[State::Handover]),
    (
        "History",
        &[State::Completed, State::Rejected, State::Canceled],
    ),
];

const KANBAN_COLUMNS: &[(&str, &str, &[State], bool, &str)] = &[
    (
        "Ideas",
        "spark · refine",
        &[State::Draft, State::Refine],
        false,
        "No idea is waiting for refinement.",
    ),
    (
        "Pitch",
        "accept or reject",
        &[State::Pitch],
        true,
        "No pitch is waiting for the consortium.",
    ),
    (
        "Committed",
        "accepted · roadmap",
        &[State::Accepted, State::Roadmap],
        true,
        "No batch is committed.",
    ),
    (
        "Active",
        "breakdown · doing",
        &[State::Breakdown, State::Doing],
        false,
        "No batch in breakdown or doing.",
    ),
    (
        "Handover",
        "batch review",
        &[State::Handover],
        true,
        "No batch is waiting for review.",
    ),
    (
        "Completed",
        "this iteration",
        &[State::Completed],
        false,
        "No batch is completed this iteration.",
    ),
];

#[derive(Clone, Copy)]
enum GateKind {
    Pitch,
    Estimate,
    Clarification,
    Handover,
}

struct Gate<'a> {
    kind: GateKind,
    card: &'a ProcessCard,
}

impl GateKind {
    fn label(self) -> &'static str {
        match self {
            Self::Pitch => "Consortium",
            Self::Estimate => "Estimate",
            Self::Clarification => "Clarification",
            Self::Handover => "Handover review",
        }
    }
}

fn gate_list(loaded: &ProcessLoaded) -> Vec<Gate<'_>> {
    loaded
        .cards
        .iter()
        .filter(|card| card.archived_at.is_none())
        .filter_map(|card| {
            let kind = match card.state {
                State::Pitch => GateKind::Pitch,
                State::Accepted if card.estimate.is_none() => GateKind::Estimate,
                State::Breakdown if loaded.open_clarification_count(&card.id) > 0 => {
                    GateKind::Clarification
                }
                State::Handover => GateKind::Handover,
                _ => return None,
            };
            Some(Gate { kind, card })
        })
        .collect()
}

fn non_task_count(loaded: &ProcessLoaded) -> usize {
    loaded
        .cards
        .iter()
        .filter(|card| card.kind != mdboard_core::process::Kind::Task && card.archived_at.is_none())
        .count()
}

fn state_count(loaded: &ProcessLoaded, state: State) -> usize {
    loaded
        .cards
        .iter()
        .filter(|card| {
            card.kind != mdboard_core::process::Kind::Task
                && card.state == state
                && card.archived_at.is_none()
        })
        .count()
}

fn is_gate(gates: &[Gate<'_>], card: &ProcessCard) -> bool {
    gates.iter().any(|gate| gate.card.id == card.id)
}

fn tasks_for<'a>(loaded: &'a ProcessLoaded, id: &str) -> Vec<&'a ProcessCard> {
    loaded
        .cards
        .iter()
        .filter(|card| card.parent.as_deref() == Some(id))
        .collect()
}

fn format_tokens(tokens: f64) -> String {
    if tokens >= 1000.0 {
        format!("{:.1}k", tokens / 1000.0)
    } else {
        format!("{tokens:.0}")
    }
}

fn card_meta(loaded: &ProcessLoaded, card: &ProcessCard) -> String {
    if let Some(estimate) = &card.estimate {
        return format!(
            "{} tokens · {:.2} agent h",
            format_tokens(estimate.tokens.weighted),
            estimate.agent_hours.weighted
        );
    }
    if card.state == State::Handover {
        let tasks = tasks_for(loaded, &card.id);
        let done = tasks
            .iter()
            .filter(|task| task.state == State::Done)
            .count();
        let round = loaded
            .reviews
            .iter()
            .filter(|review| review.batch == card.id)
            .map(|review| review.round)
            .max()
            .unwrap_or(1);
        return format!("{done} / {} tasks done · round {round}", tasks.len());
    }
    if card.state == State::Completed {
        let date = card.updated_at.get(..10).unwrap_or(&card.updated_at);
        return format!("completed {date}");
    }
    if card.kind == mdboard_core::process::Kind::Batch && card.state == State::Accepted {
        return "accepted · not estimated".to_owned();
    }
    format!("{} · {}", card.kind, card.state)
}

fn board_panel(loaded: &ProcessLoaded, gates: &[Gate<'_>]) -> String {
    let mut html = format!(
        r#"<a class="panel-item is-active" href="/board.html">All cards <span class="count">{}</span></a><span class="panel-item is-needs"><span>Needs you</span><span class="count">{}</span></span>"#,
        non_task_count(loaded),
        gates.len()
    );
    html.push_str("<div class=\"auth-only\">");
    for (group, states) in PANEL_GROUPS {
        html.push_str(&format!("<div class=\"panel-group\">{group}</div>"));
        for state in *states {
            html.push_str(&format!(
                "<a class=\"panel-item\" href=\"/board.html#state={}\"><span>{}</span><span class=\"count\">{}</span></a>",
                state.as_str(),
                state.as_str(),
                state_count(loaded, *state)
            ));
        }
    }
    html.push_str("</div>");
    html.push_str(&format!(
        "<div class=\"anon-only\"><a class=\"panel-item\" href=\"/board.html#state=open\">Open gates <span class=\"count\">{}</span></a></div>",
        gates.len()
    ));
    html
}

fn gate_card(loaded: &ProcessLoaded, gate: &Gate<'_>) -> String {
    let card = gate.card;
    let slug = card.id.trim_start_matches('#');
    let context = match gate.kind {
        GateKind::Pitch => {
            let tokens = card.estimate.as_ref().map_or_else(
                || "not estimated".to_owned(),
                |estimate| format!("{} tokens", format_tokens(estimate.tokens.weighted)),
            );
            format!("{} · pitch · {tokens}", card.id)
        }
        GateKind::Estimate => format!(
            "{} · accepted · roadmap has {} free slots",
            card.id,
            loaded
                .roadmap_limit()
                .saturating_sub(loaded.roadmap_count())
        ),
        GateKind::Clarification => format!("{} · breakdown · question open", card.id),
        GateKind::Handover => {
            let tasks = tasks_for(loaded, &card.id);
            let done = tasks
                .iter()
                .filter(|task| task.state == State::Done)
                .count();
            let round = loaded
                .reviews
                .iter()
                .filter(|review| review.batch == card.id)
                .map(|review| review.round)
                .max()
                .unwrap_or(1);
            format!(
                "{} · {done} / {} tasks done · round {round}",
                card.id,
                tasks.len()
            )
        }
    };
    let actions = match gate.kind {
        GateKind::Pitch => format!(
            "<button class=\"btn btn-primary\" onclick=\"acceptIdea('{}')\">Accept</button><button class=\"btn btn-secondary\" onclick=\"rejectIdea('{}')\">Reject</button>",
            escape(&card.id),
            escape(&card.id)
        ),
        GateKind::Estimate => format!(
            "<a class=\"btn btn-primary\" href=\"/cards/{slug}.html\">Estimate</a><a class=\"btn btn-secondary\" href=\"/cards/{slug}.html\">Open card</a>"
        ),
        GateKind::Clarification => format!(
            "<a class=\"btn btn-primary\" href=\"/cards/{slug}.html\">Answer</a><a class=\"btn btn-secondary\" href=\"/cards/{slug}.html\">Open card</a>"
        ),
        GateKind::Handover => format!(
            "<a class=\"btn btn-primary\" href=\"/cards/{slug}.html#review\">Review batch</a><button class=\"btn btn-secondary\" onclick=\"requestChanges('{}')\">Request changes</button>",
            escape(&card.id)
        ),
    };
    format!(
        "<article class=\"gate-card\"><p class=\"gate-kind\">{}</p><h3 class=\"gate-title\">{}</h3><p class=\"gate-meta\">{}</p><div class=\"gate-actions\">{}</div></article>",
        gate.kind.label(),
        escape(&card.title),
        context,
        actions
    )
}

fn gate_callout(loaded: &ProcessLoaded, gates: &[Gate<'_>]) -> String {
    let signed_in = if gates.is_empty() {
        "<div class=\"gate-grid\"><p class=\"gate-empty\">Nothing needs you right now.</p></div>"
            .to_owned()
    } else {
        let mut html = String::from("<div class=\"gate-grid\">");
        for gate in gates {
            html.push_str(&gate_card(loaded, gate));
        }
        html.push_str("</div>");
        html
    };
    let signed_out = if gates.is_empty() {
        "<p>No human gates are open. Sign in to follow the board.</p>".to_owned()
    } else {
        let ids = gates
            .iter()
            .map(|gate| gate.card.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        format!("<p>{ids} wait for a human decision. Sign in as the owner or a member to act.</p>")
    };
    format!(
        "<section class=\"needs auth-only\"><div class=\"needs-head\"><span>Needs you · {}</span><span class=\"needs-note\">Only the owner can record these decisions</span></div>{signed_in}</section><section class=\"needs anon-only\"><div class=\"needs-head\"><span>{} human gates are open</span></div>{signed_out}<a class=\"btn btn-primary btn-lg\" href=\"/login.html\">Sign in to review</a></section>",
        gates.len(),
        gates.len()
    )
}

fn kanban_card(loaded: &ProcessLoaded, card: &ProcessCard, gates: &[Gate<'_>]) -> String {
    let slug = card.id.trim_start_matches('#');
    let pill = if is_gate(gates, card) {
        "<span class=\"pill auth-only\">your call</span><span class=\"pill anon-only\">waiting on owner</span>"
    } else {
        ""
    };
    format!(
        "<a class=\"kcard\" data-state=\"{}\" data-gate=\"{}\" href=\"/cards/{slug}.html\"><div class=\"kcard-id\"><span>{}</span>{pill}</div><div class=\"kcard-title\">{}</div><div class=\"kcard-meta\">{}</div></a>",
        card.state,
        if is_gate(gates, card) { "1" } else { "0" },
        escape(&card.id),
        escape(&card.title),
        escape(&card_meta(loaded, card))
    )
}

fn kanban(loaded: &ProcessLoaded, gates: &[Gate<'_>]) -> String {
    let mut html = String::from("<h2>Lifecycle</h2><div class=\"kanban\">");
    for (name, sub, states, gate_column, empty) in KANBAN_COLUMNS {
        let cards = loaded
            .cards
            .iter()
            .filter(|card| {
                card.kind != mdboard_core::process::Kind::Task
                    && states.contains(&card.state)
                    && card.archived_at.is_none()
            })
            .collect::<Vec<_>>();
        let sub = if *gate_column {
            format!(
                "<span class=\"kcolumn-sub gate\"><span class=\"gate-dot\"></span>{sub} · human gate</span>"
            )
        } else {
            format!("<span class=\"kcolumn-sub\">{sub}</span>")
        };
        html.push_str(&format!(
            "<section class=\"kcolumn\"><header class=\"kcolumn-head\"><h3>{name}</h3><span class=\"count\">{}</span></header>{sub}",
            cards.len()
        ));
        if cards.is_empty() {
            html.push_str(&format!("<p class=\"empty-column\">{empty}</p>"));
        } else {
            for card in cards {
                html.push_str(&kanban_card(loaded, card, gates));
            }
        }
        html.push_str("</section>");
    }
    html.push_str("</div>");
    html
}

fn standing_orders_panel(loaded: &ProcessLoaded) -> String {
    let active = loaded.active_orders();
    let mut html = String::from("<section class=\"panel\"><h2>Standing orders</h2>");
    if active.is_empty() {
        html.push_str("<p class=\"muted\">No active standing orders.</p>");
    } else {
        html.push_str("<ul>");
        for order in active {
            html.push_str(&format!(
                "<li><strong>{}</strong> — {}</li>",
                escape(&order.id),
                escape(&order.title)
            ));
        }
        html.push_str("</ul>");
    }
    html.push_str("</section>");
    html
}

fn governance_panel(loaded: &ProcessLoaded) -> String {
    let guardian = &loaded.policy.guardian;
    let stakeholders = &loaded.policy.stakeholders;
    let mut html = String::from("<section class=\"panel governance\"><h2>Governance</h2>");
    html.push_str(&format!(
        "<p><strong>Guardian:</strong> {} &lt;{}&gt; · {} co-guardian(s)</p>",
        escape(&guardian.name),
        escape(&guardian.email),
        guardian.co_guardians.len()
    ));
    html.push_str(&format!(
        "<p><strong>Stakeholders:</strong> {}/{} seats · quorum {}</p>",
        stakeholders.total_seats(),
        stakeholders.max_seats,
        stakeholders.quorum_seats
    ));
    html.push_str(&format!(
        "<p><strong>SSH authentication:</strong> {}</p>",
        if process::has_ssh_keys(loaded) {
            "enabled"
        } else {
            "disabled"
        }
    ));
    if stakeholders.members.is_empty() {
        html.push_str("<p class=\"muted\">No stakeholder seats configured; proposals are accepted without a vote.</p>");
    } else {
        html.push_str("<table class=\"table\"><thead><tr><th>Stakeholder</th><th>Seats</th><th>Seat until</th></tr></thead><tbody>");
        for member in &stakeholders.members {
            html.push_str(&format!(
                "<tr><td>{} &lt;{}&gt;</td><td>{}</td><td>{}</td></tr>",
                escape(&member.name),
                escape(&member.email),
                member.seats,
                escape(&member.seat_until)
            ));
        }
        html.push_str("</tbody></table>");
    }
    if let Some(active) = loaded.active_batches().next()
        && let Some(vote) = loaded.latest_vote_for(&active.id)
    {
        html.push_str(&format!(
            "<p><strong>Latest vote:</strong> {} — {:?} · {} of {} seats counted</p>",
            escape(&vote.id),
            vote.status,
            vote.counted_seats,
            vote.allocated_seats
        ));
    }
    html.push_str("</section>");
    html
}

fn vote_panel(vote: &mdboard_core::process::Vote) -> String {
    let mut html = format!(
        "<section class=\"panel\"><h2>Stakeholder vote</h2><p><strong>{}</strong> — {:?} · {} of {} seats counted · quorum {}</p>",
        escape(&vote.id),
        vote.status,
        vote.counted_seats,
        vote.allocated_seats,
        vote.quorum_seats
    );
    if let Some(reason) = &vote.reason {
        html.push_str(&format!("<p class=\"muted\">{}</p>", escape(reason)));
    }
    if let Some(decision) = &vote.decision {
        html.push_str(&format!("<p class=\"muted\">{}</p>", escape(decision)));
    }
    html.push_str("<table class=\"table\"><thead><tr><th>Stakeholder</th><th>Seats</th><th>Result</th></tr></thead><tbody>");
    for item in &vote.votes {
        html.push_str(&format!(
            "<tr><td>{} &lt;{}&gt;</td><td>{}</td><td>{:?}</td></tr>",
            escape(&item.name),
            escape(&item.email),
            item.seats,
            item.choice
        ));
    }
    html.push_str("</tbody></table></section>");
    html
}

fn board_page(loaded: &ProcessLoaded) -> String {
    let gates = gate_list(loaded);
    let panel = board_panel(loaded, &gates);
    let count = non_task_count(loaded);
    let mut body =
        format!("<h1>Board</h1><p class=\"lede\">{count} cards in the open upstream iteration.");
    if let Some(active) = loaded.active_batches().next() {
        body.push_str(&format!(
            " Batch {} is in {}.",
            escape(&active.id),
            active.state
        ));
    }
    body.push_str("</p>");
    body.push_str(&gate_callout(loaded, &gates));
    body.push_str(&governance_panel(loaded));
    body.push_str(&standing_orders_panel(loaded));
    body.push_str(&kanban(loaded, &gates));
    layout_with_panel("Board", loaded, "board", "Board", &panel, &body)
}

fn roadmap_page(loaded: &ProcessLoaded) -> String {
    let mut body = String::new();
    body.push_str(
        "<div class=\"page-head\"><p class=\"eyebrow\">Committed</p><h1>Roadmap</h1></div>",
    );
    body.push_str(&format!(
        "<p class=\"muted\">{} of {} slots · estimated €{:.4} ({})</p>",
        loaded.roadmap_count(),
        loaded.roadmap_limit(),
        loaded.estimated_cost_eur(),
        escape(&loaded.policy.costs.subscription)
    ));
    body.push_str("<table class=\"table\"><thead><tr><th>ID</th><th>Title</th><th>State</th><th>Tokens</th><th>Agent hours</th><th>Estimated €</th></tr></thead><tbody>");
    for card in loaded
        .cards
        .iter()
        .filter(|card| card.state == State::Roadmap)
    {
        let (tokens, hours, cost) = card.estimate.as_ref().map_or((0.0, 0.0, 0.0), |estimate| {
            (
                estimate.tokens.weighted,
                estimate.agent_hours.weighted,
                estimate.tokens.weighted / 1_000_000.0 * loaded.policy.costs.eur_per_million_tokens,
            )
        });
        body.push_str(&format!("<tr><td><a href=\"/cards/{}.html\">{}</a></td><td>{}</td><td>{}</td><td>{tokens:.0}</td><td>{hours:.2}</td><td>€{cost:.4}</td></tr>", escape(card.id.trim_start_matches('#')), escape(&card.id), escape(&card.title), card.state));
    }
    body.push_str("</tbody></table>");
    layout("Roadmap", loaded, &body)
}

fn card_page(loaded: &ProcessLoaded, card: &ProcessCard) -> String {
    let mut body = String::new();
    body.push_str(&format!("<div class=\"page-head\"><p class=\"eyebrow\">{} · {}</p><h1>{}</h1><p class=\"muted\">{}</p></div>", card.kind, card.state, escape(&card.title), escape(&card.id)));
    if card.archived_at.is_some() {
        body.push_str("<section class=\"panel\"><h2>Archived</h2><p class=\"muted\">This card is archived and hidden from active views.</p></section>");
    }
    if card.state == State::Obsolete {
        body.push_str("<section class=\"panel\"><h2>Obsolete</h2><p class=\"muted\">This task is resolved but not delivered.</p></section>");
    }
    body.push_str(&format!(
        "<p class=\"muted\">Intent owner: {}</p>",
        escape(&card.intent_owner)
    ));
    if let Some(estimate) = &card.estimate {
        body.push_str(&format!(
            "<p class=\"muted\">Estimate: {:.0} tokens · {:.2} agent hours</p>",
            estimate.tokens.weighted, estimate.agent_hours.weighted
        ));
    }
    body.push_str("<section class=\"panel\"><h2>Intent</h2><pre class=\"body\">");
    body.push_str(&escape(&card.body));
    body.push_str("</pre></section>");
    let applicable = loaded.applicable_orders(card);
    if !applicable.is_empty() {
        body.push_str("<section class=\"panel\"><h2>Standing orders</h2><ul>");
        for order in applicable {
            body.push_str(&format!(
                "<li><strong>{}</strong> — {}</li>",
                escape(&order.id),
                escape(&order.title)
            ));
        }
        body.push_str("</ul></section>");
    }

    if card.kind == mdboard_core::process::Kind::Proposal && card.state == State::Pitch {
        if let Some(vote) = loaded.latest_vote_for(&card.id) {
            body.push_str(&vote_panel(vote));
        } else if !loaded.policy.stakeholders.members.is_empty() {
            body.push_str("<section class=\"panel\"><h2>Stakeholder vote</h2><p class=\"muted\">No vote has been opened. A passed vote is required before acceptance.</p></section>");
        }
        body.push_str(&format!("<section class=\"panel\"><h2>Consortium decision</h2><button onclick=\"acceptIdea('{}')\">Accept</button><button class=\"secondary\" onclick=\"rejectIdea('{}')\">Reject</button></section>", escape(&card.id), escape(&card.id)));
    }

    if card.kind == mdboard_core::process::Kind::Batch {
        let tasks = loaded
            .cards
            .iter()
            .filter(|task| task.parent.as_deref() == Some(card.id.as_str()))
            .collect::<Vec<_>>();
        body.push_str("<section class=\"panel\"><h2>Tasks</h2>");
        if tasks.is_empty() {
            body.push_str("<p class=\"muted\">No tasks.</p>");
        } else {
            body.push_str("<table class=\"table\"><thead><tr><th>ID</th><th>Title</th><th>State</th></tr></thead><tbody>");
            for task in tasks {
                body.push_str(&format!(
                    "<tr><td><a href=\"/cards/{}.html\">{}</a></td><td>{}</td><td>{}</td></tr>",
                    escape(task.id.trim_start_matches('#')),
                    escape(&task.id),
                    escape(&task.title),
                    task.state
                ));
            }
            body.push_str("</tbody></table>");
        }
        body.push_str("</section>");

        let reviews = loaded
            .reviews
            .iter()
            .filter(|review| review.batch == card.id)
            .collect::<Vec<_>>();
        body.push_str("<section class=\"panel\"><h2>Reviews</h2>");
        if reviews.is_empty() {
            body.push_str("<p class=\"muted\">No review artifacts yet.</p>");
        } else {
            for review in reviews {
                body.push_str(&format!(
                    "<p><strong>{}</strong> by {} — {:?} (round {})</p><ul>",
                    escape(&review.id),
                    escape(&review.reviewer),
                    review.verdict,
                    review.round
                ));
                for finding in &review.findings {
                    body.push_str(&format!(
                        "<li>{}: {}</li>",
                        escape(&finding.id),
                        escape(&finding.text)
                    ));
                }
                body.push_str("</ul>");
            }
        }
        body.push_str("</section>");

        if card.state == State::Handover {
            body.push_str(&format!("<section class=\"panel\"><h2>Handover review</h2><p class=\"muted\">Review the batch as a whole. The reviewer must differ from the intent owner.</p><label>Reviewer <input id=\"reviewer\" value=\"member\"></label><label>Authored by <input id=\"authored_by\" value=\"member\"></label><label>Round <input id=\"round\" type=\"number\" value=\"1\" min=\"1\"></label><label>Verdict <select id=\"verdict\"><option value=\"accepted\">accepted</option><option value=\"changes-required\">changes-required</option></select></label><label>Findings (one per line)<textarea id=\"findings\" rows=\"4\"></textarea></label><button onclick=\"reviewBatch('{}')\">Record review</button><button class=\"secondary\" onclick=\"requestChanges('{}')\">Request changes</button></section>", escape(&card.id), escape(&card.id)));
        }
        if card.state == State::Handover {
            body.push_str(&format!("<section class=\"panel\"><h2>Owner acceptance</h2><button onclick=\"completeBatch('{}')\">Accept and complete</button></section>", escape(&card.id)));
        }
        if card.state == State::Blocked {
            body.push_str("<section class=\"panel\"><h2>Blocked</h2><p>");
            body.push_str(&escape(card.blocked_reason.as_deref().unwrap_or("")));
            body.push_str("</p></section>");
        }
    }

    body.push_str("<section class=\"panel\"><h2>Event history</h2><ul class=\"link-list\">");
    for event in loaded.history(&card.id) {
        body.push_str(&format!(
            "<li>{} — {} ({} → {})</li>",
            escape(&event.at),
            escape(&event.action),
            event
                .from
                .map_or_else(|| "-".to_owned(), |state| state.to_string()),
            event
                .to
                .map_or_else(|| "-".to_owned(), |state| state.to_string())
        ));
    }
    body.push_str("</ul></section>");
    layout(&card.title, loaded, &body)
}

fn meetings_page(loaded: &ProcessLoaded) -> String {
    let mut body = String::new();
    body.push_str(
        "<div class=\"page-head\"><p class=\"eyebrow\">Human gates</p><h1>Meetings</h1></div>",
    );
    if loaded.meetings.is_empty() {
        body.push_str("<p class=\"muted\">No meetings.</p>");
    } else {
        for meeting in &loaded.meetings {
            body.push_str(&format!("<section class=\"panel\"><h2>{} <span class=\"badge\">{:?}</span></h2><p>{}</p><p class=\"muted\">{:?} · {}</p>", escape(&meeting.id), meeting.status, escape(&meeting.trigger), meeting.kind, escape(&meeting.linked_cards.join(", "))));
            for question in &meeting.questions {
                body.push_str(&format!(
                    "<p>{}: {} {}</p>",
                    escape(&question.id),
                    escape(&question.text),
                    if question.resolved { "✓" } else { "open" }
                ));
            }
            if matches!(
                meeting.status,
                mdboard_core::process::MeetingStatus::Pending
                    | mdboard_core::process::MeetingStatus::Deferred
            ) {
                body.push_str(&format!("<button onclick=\"meetingAction('{}','accept')\">Accept</button><button class=\"secondary\" onclick=\"meetingAction('{}','defer')\">Defer</button><button class=\"secondary\" onclick=\"meetingAction('{}','dismiss')\">Dismiss</button>", escape(&meeting.id), escape(&meeting.id), escape(&meeting.id)));
            }
            body.push_str("</section>");
        }
    }
    layout("Meetings", loaded, &body)
}

fn login_page() -> String {
    "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Sign in · mdBoard</title><link rel=\"stylesheet\" href=\"/assets/style.css\"><script defer src=\"/assets/process.js\"></script></head><body><main class=\"login\"><h1>Sign in</h1><p class=\"muted\">Use the one-time token printed by <code>mdboard serve</code>.</p><form onsubmit=\"login(event)\"><label>Token <input id=\"token\" autocomplete=\"off\"></label><button type=\"submit\">Sign in</button></form></main></body></html>".to_owned()
}

fn card_tile(card: &ProcessCard) -> String {
    format!(
        "<a class=\"card\" href=\"/cards/{}.html\"><span class=\"badge\">{}</span><h3>{}</h3><p class=\"muted\">{}</p></a>",
        escape(card.id.trim_start_matches('#')),
        card.state,
        escape(&card.title),
        card.kind
    )
}

fn board_value(loaded: &ProcessLoaded) -> Value {
    json!({
        "schema": 3,
        "upstream": loaded.upstream.as_ref().map(|iteration| json!({"iteration": iteration.iteration, "status": iteration.status, "base": iteration.base, "writer": iteration.writer})),
        "roadmap": {"count": loaded.roadmap_count(), "limit": loaded.roadmap_limit(), "estimated_cost_eur": loaded.estimated_cost_eur()},
        "standing_orders": loaded.active_orders().iter().map(|order| json!({"id": order.id, "title": order.title, "scope": order.scope})).collect::<Vec<_>>(),
        "governance": {
            "guardian": {"name": loaded.policy.guardian.name, "email": loaded.policy.guardian.email},
            "ssh_enabled": process::has_ssh_keys(loaded),
            "co_guardians": loaded.policy.guardian.co_guardians.len(),
            "stakeholders": loaded.policy.stakeholders.members.len(),
            "allocated_seats": loaded.policy.stakeholders.total_seats(),
            "max_seats": loaded.policy.stakeholders.max_seats,
            "quorum_seats": loaded.policy.stakeholders.quorum_seats,
        },
        "active_batch": loaded.active_batches().next().map(|card| card.id.clone()),
        "pending_meetings": loaded.pending_meetings().iter().map(|meeting| meeting.id.clone()).collect::<Vec<_>>(),
        "cards": loaded.cards.iter().filter(|card| card.archived_at.is_none()).map(|card| json!({"id": card.id, "kind": card.kind, "state": card.state, "title": card.title, "intent_owner": card.intent_owner})).collect::<Vec<_>>(),
    })
}

fn card_value(loaded: &ProcessLoaded, card: &ProcessCard) -> Value {
    let tasks = loaded
        .cards
        .iter()
        .filter(|task| task.parent.as_deref() == Some(card.id.as_str()))
        .collect::<Vec<_>>();
    let reviews = loaded
        .reviews
        .iter()
        .filter(|review| review.batch == card.id)
        .collect::<Vec<_>>();
    json!({
        "card": card,
        "tasks": tasks,
        "reviews": reviews,
        "latest_vote": loaded.latest_vote_for(&card.id),
        "standing_orders": loaded.applicable_orders(card),
        "estimated_cost_eur": card.estimate.as_ref().map(|estimate| estimate.tokens.weighted / 1_000_000.0 * loaded.policy.costs.eur_per_million_tokens),
    })
}

fn schema_value() -> Value {
    json!({
        "schema": 3,
        "kinds": mdboard_core::process::Kind::ALL.iter().map(|kind| json!({"kind": kind.as_str(), "states": kind.states().iter().map(|state| state.as_str()).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "transitions": mdboard_core::process::TRANSITIONS.iter().map(|rule| json!({"action": rule.action.as_str(), "from": rule.from.as_str(), "to": rule.to.as_str(), "from_kind": rule.from_kind.as_str(), "authority": rule.authority.as_str(), "guard": rule.guard})).collect::<Vec<_>>(),
    })
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".to_owned())
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn board_json(
    AxumState(state): AxumState<AppState>,
) -> std::result::Result<Json<Value>, ApiError> {
    let loaded = ProcessLoaded::load(&state.root).map_err(ApiError::from)?;
    Ok(Json(board_value(&loaded)))
}

async fn card_json(
    AxumState(state): AxumState<AppState>,
    AxumPath(id): AxumPath<String>,
) -> std::result::Result<Json<Value>, ApiError> {
    let loaded = ProcessLoaded::load(&state.root).map_err(ApiError::from)?;
    let id = id.strip_suffix(".json").unwrap_or(&id);
    let card = loaded
        .find(id)
        .or_else(|| loaded.find(&format!("#{id}")))
        .ok_or_else(|| ApiError::new(axum::http::StatusCode::NOT_FOUND, "card not found"))?;
    Ok(Json(card_value(&loaded, card)))
}

async fn meetings_json(
    AxumState(state): AxumState<AppState>,
) -> std::result::Result<Json<Value>, ApiError> {
    let loaded = ProcessLoaded::load(&state.root).map_err(ApiError::from)?;
    Ok(Json(
        serde_json::to_value(&loaded.meetings).unwrap_or(Value::Null),
    ))
}

async fn schema_json() -> Json<Value> {
    Json(schema_value())
}

#[derive(Deserialize)]
struct FindingBody {
    id: String,
    text: String,
}
#[derive(Deserialize)]
struct ReviewBody {
    reviewer: String,
    authored_by: String,
    verdict: String,
    #[serde(default)]
    findings: Vec<FindingBody>,
    #[serde(default)]
    round: Option<u32>,
}
#[derive(Deserialize, Default)]
struct AuthBody {
    #[serde(default)]
    authorised_by: Option<String>,
    #[serde(default)]
    authorization: Option<String>,
}
#[derive(Deserialize)]
struct ReasonBody {
    reason: String,
}

async fn accept_idea(
    AxumState(state): AxumState<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> std::result::Result<Json<Value>, ApiError> {
    let session = require_write(&state, &headers)?;
    let card = process::transition(
        &state.root,
        &session.actor,
        &id,
        Action::AcceptIdea,
        None,
        None,
        None,
        None,
    )
    .map_err(ApiError::from)?;
    crate::rebuild(&state)?;
    Ok(Json(
        json!({"ok": true, "card": card.id, "state": card.state}),
    ))
}

async fn reject_idea(
    AxumState(state): AxumState<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<ReasonBody>,
) -> std::result::Result<Json<Value>, ApiError> {
    let session = require_write(&state, &headers)?;
    let card = process::transition(
        &state.root,
        &session.actor,
        &id,
        Action::RejectIdea,
        Some(&body.reason),
        None,
        None,
        None,
    )
    .map_err(ApiError::from)?;
    crate::rebuild(&state)?;
    Ok(Json(
        json!({"ok": true, "card": card.id, "state": card.state}),
    ))
}

async fn review_batch(
    AxumState(state): AxumState<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<ReviewBody>,
) -> std::result::Result<Json<Value>, ApiError> {
    let session = require_write(&state, &headers)?;
    let verdict = match body.verdict.as_str() {
        "accepted" => Verdict::Accepted,
        "changes-required" => Verdict::ChangesRequired,
        _ => {
            return Err(ApiError::new(
                axum::http::StatusCode::BAD_REQUEST,
                "unknown verdict",
            ));
        }
    };
    let findings = body
        .findings
        .into_iter()
        .map(|finding| (finding.id, finding.text))
        .collect::<Vec<_>>();
    let artifact = process::review(
        &state.root,
        &session.actor,
        &id,
        &body.reviewer,
        &body.authored_by,
        verdict,
        &findings,
        body.round.unwrap_or(1),
    )
    .map_err(ApiError::from)?;
    crate::rebuild(&state)?;
    Ok(Json(json!({"ok": true, "review": artifact.id})))
}

async fn complete_batch(
    AxumState(state): AxumState<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<AuthBody>,
) -> std::result::Result<Json<Value>, ApiError> {
    let session = require_write(&state, &headers)?;
    let card = process::transition(
        &state.root,
        &session.actor,
        &id,
        Action::Complete,
        None,
        body.authorised_by.as_deref(),
        body.authorization.as_deref(),
        None,
    )
    .map_err(ApiError::from)?;
    crate::rebuild(&state)?;
    Ok(Json(
        json!({"ok": true, "card": card.id, "state": card.state}),
    ))
}

async fn request_changes(
    AxumState(state): AxumState<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<ReasonBody>,
) -> std::result::Result<Json<Value>, ApiError> {
    let session = require_write(&state, &headers)?;
    let card = process::transition(
        &state.root,
        &session.actor,
        &id,
        Action::RequestChanges,
        Some(&body.reason),
        None,
        None,
        None,
    )
    .map_err(ApiError::from)?;
    crate::rebuild(&state)?;
    Ok(Json(
        json!({"ok": true, "card": card.id, "state": card.state}),
    ))
}

async fn meeting_accept(
    AxumState(state): AxumState<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
) -> std::result::Result<Json<Value>, ApiError> {
    let session = require_write(&state, &headers)?;
    process::meeting_accept(&state.root, &session.actor, &id).map_err(ApiError::from)?;
    crate::rebuild(&state)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct DeferBody {
    until: String,
}
async fn meeting_defer(
    AxumState(state): AxumState<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<DeferBody>,
) -> std::result::Result<Json<Value>, ApiError> {
    let session = require_write(&state, &headers)?;
    process::meeting_defer(&state.root, &session.actor, &id, &body.until)
        .map_err(ApiError::from)?;
    crate::rebuild(&state)?;
    Ok(Json(json!({"ok": true})))
}

async fn meeting_dismiss(
    AxumState(state): AxumState<AppState>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<ReasonBody>,
) -> std::result::Result<Json<Value>, ApiError> {
    let session = require_write(&state, &headers)?;
    process::meeting_dismiss(&state.root, &session.actor, &id, &body.reason)
        .map_err(ApiError::from)?;
    crate::rebuild(&state)?;
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> ProcessLoaded {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/minimal");
        ProcessLoaded::load(&root).expect("load fixture")
    }

    #[test]
    fn board_uses_the_new_shell_and_kanban() {
        let html = board_page(&fixture());
        assert!(html.contains("class=\"app\" data-auth=\"anon\""));
        assert!(html.contains("class=\"rail\""));
        assert!(html.contains("class=\"panel\""));
        assert!(html.contains("class=\"kanban\""));
        assert!(html.contains("class=\"needs auth-only\""));
        assert!(html.contains("class=\"needs anon-only\""));
        assert!(html.contains("class=\"kcard\""));
        assert!(!html.contains("style="));
        assert!(html.contains("The HTML board is a view; Git is truth."));
        assert!(html.contains("Governance"));
        assert!(html.contains("No stakeholder seats configured"));
        assert!(html.contains("Standing orders"));
        assert!(html.contains("No active standing orders"));
    }

    #[test]
    fn board_json_exposes_governance_and_schema_3() {
        let value = board_value(&fixture());
        assert_eq!(value["schema"], 3);
        assert_eq!(value["governance"]["max_seats"], 5);
        assert_eq!(value["governance"]["quorum_seats"], 3);
        assert_eq!(value["governance"]["stakeholders"], 0);
    }

    #[test]
    fn panel_and_kanban_mappings_cover_their_states() {
        let mut panel_states = PANEL_GROUPS
            .iter()
            .flat_map(|(_, states)| states.iter().copied())
            .collect::<Vec<_>>();
        panel_states.sort();
        panel_states.dedup();
        assert_eq!(panel_states.len(), 12);
        assert!(panel_states.contains(&State::Draft));
        assert!(panel_states.contains(&State::Canceled));
        assert!(
            KANBAN_COLUMNS
                .iter()
                .any(|(name, _, _, _, _)| *name == "Ideas")
        );
        assert!(
            KANBAN_COLUMNS
                .iter()
                .any(|(name, _, _, _, _)| *name == "Completed")
        );
    }

    #[test]
    fn tokens_use_compact_display() {
        assert_eq!(format_tokens(9_333.0), "9.3k");
        assert_eq!(format_tokens(42.0), "42");
    }
}
