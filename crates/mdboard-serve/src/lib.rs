//! Local HTML board and JSON API for the 1.0 process model.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use axum::Json;
use axum::extract::State as AxumState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use mdboard_core::Actors;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast;
use uuid::Uuid;

pub mod process;

#[derive(Debug, Clone)]
pub struct ServeOptions {
    pub host: String,
    pub port: u16,
    pub open: bool,
    pub watch: bool,
    /// Actor id used by the local one-time token. `None` keeps the server read-only.
    pub actor: Option<String>,
}

impl Default for ServeOptions {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port: 3000,
            open: false,
            watch: true,
            actor: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BuildOutput {
    pub site_dir: PathBuf,
    pub cards: usize,
    pub findings: usize,
    pub upstream: String,
}

/// Build the process site and Markdown views.
pub fn build(root: &Path) -> Result<BuildOutput> {
    process::build(root)
}

/// Watch the process directory and rebuild on change.
pub fn watch(root: &Path) -> Result<()> {
    process::watch(root)
}

/// Serve the static site, JSON API and live-reload WebSocket.
pub fn serve(root: &Path, options: &ServeOptions) -> Result<()> {
    let first = build(root)?;
    report(&first);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("cannot create the async runtime")?;
    runtime.block_on(serve_async(root.to_path_buf(), options.clone()))
}

#[derive(Clone)]
struct AppState {
    root: PathBuf,
    site_dir: PathBuf,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    token: Arc<Mutex<Option<LoginToken>>>,
    actors: Actors,
    sender: broadcast::Sender<()>,
}

#[derive(Clone)]
struct Session {
    actor: String,
    roles: Vec<String>,
    csrf: String,
    expires: DateTime<Utc>,
}

#[derive(Clone)]
struct LoginToken {
    value: String,
    actor: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    token: String,
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}

impl From<mdboard_actions::process::ActionError> for ApiError {
    fn from(error: mdboard_actions::process::ActionError) -> Self {
        use mdboard_actions::process::ActionError as E;
        let status = match error {
            E::NotFound(_) => StatusCode::NOT_FOUND,
            E::Forbidden(_) => StatusCode::FORBIDDEN,
            E::Conflict(_) => StatusCode::CONFLICT,
            E::BadRequest(_) => StatusCode::BAD_REQUEST,
            E::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: error.to_string(),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

async fn serve_async(root: PathBuf, options: ServeOptions) -> Result<()> {
    let loaded = mdboard_driver::process::ProcessLoaded::load(&root)?;
    let actor_path = loaded.process_root().join("actors.toml");
    let actors = Actors::load_optional(&actor_path)?;
    let login_token = match &options.actor {
        Some(actor) => {
            if actors.get(actor).is_none() {
                anyhow::bail!("actor `{actor}` is not defined in {}", actor_path.display());
            }
            Some(LoginToken {
                value: Uuid::new_v4().simple().to_string(),
                actor: actor.clone(),
            })
        }
        None => None,
    };

    let (sender, _) = broadcast::channel::<()>(16);
    let site_dir = loaded.process_root().join("site");
    let state = AppState {
        root: root.clone(),
        site_dir,
        sessions: Arc::new(Mutex::new(HashMap::new())),
        token: Arc::new(Mutex::new(login_token.clone())),
        actors,
        sender: sender.clone(),
    };

    if options.watch {
        process::spawn_watcher(root.clone(), sender)?;
    }

    let address: SocketAddr = format!("{}:{}", options.host, options.port)
        .parse()
        .context("invalid host/port")?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("cannot bind {address}"))?;
    let url = format!("http://{address}");
    println!("mdboard serve: {url}");
    if let Some(token) = &login_token {
        println!(
            "mdboard auth token (actor `{}`): {}",
            token.actor, token.value
        );
        println!("mdboard sign-in: {url}/login.html");
    } else {
        println!("mdboard: read-only (pass --actor <id> to enable writes)");
    }
    if options.open
        && let Err(error) = open::that(format!("{url}/login.html"))
    {
        eprintln!("cannot open the browser: {error}");
    }

    let app = process::router(state);
    axum::serve(listener, app).await.context("server failed")?;
    Ok(())
}

async fn login(
    AxumState(state): AxumState<AppState>,
    Json(request): Json<LoginRequest>,
) -> std::result::Result<Response, ApiError> {
    let mut token = state
        .token
        .lock()
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "token lock poisoned"))?;
    let Some(current) = token.as_ref() else {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "write access is disabled; start serve with --actor <id>",
        ));
    };
    if current.value != request.token.trim() {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "invalid token"));
    }
    let actor = current.actor.clone();
    *token = None;
    drop(token);

    let actor_config = state
        .actors
        .get(&actor)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "actor is not configured"))?;
    let roles = if actor_config.roles.is_empty() {
        vec!["viewer".to_owned()]
    } else {
        actor_config.roles.clone()
    };
    let session_id = Uuid::new_v4().simple().to_string();
    let csrf = Uuid::new_v4().simple().to_string();
    let session = Session {
        actor: actor.clone(),
        roles: roles.clone(),
        csrf: csrf.clone(),
        expires: Utc::now() + ChronoDuration::hours(8),
    };
    state
        .sessions
        .lock()
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "session lock poisoned"))?
        .insert(session_id.clone(), session);

    let cookie = format!("mdboard_session={session_id}; HttpOnly; SameSite=Strict; Path=/");
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(json!({"actor": actor, "roles": roles, "role": roles.first(), "csrf": csrf})),
    )
        .into_response())
}

async fn logout(AxumState(state): AxumState<AppState>, headers: HeaderMap) -> Response {
    if let Some(session_id) = cookie(&headers, "mdboard_session")
        && let Ok(mut sessions) = state.sessions.lock()
    {
        sessions.remove(&session_id);
    }
    (
        [(
            header::SET_COOKIE,
            "mdboard_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0".to_owned(),
        )],
        Json(json!({"ok": true})),
    )
        .into_response()
}

async fn me(
    AxumState(state): AxumState<AppState>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    let session = require_session(&state, &headers)?;
    Ok(Json(json!({
        "actor": session.actor,
        "roles": session.roles,
        "role": session.roles.first(),
        "csrf": session.csrf,
    }))
    .into_response())
}

fn require_write(state: &AppState, headers: &HeaderMap) -> std::result::Result<Session, ApiError> {
    let session = require_session(state, headers)?;
    let csrf = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if csrf != session.csrf {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "invalid CSRF token"));
    }
    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        let host = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let http = format!("http://{host}");
        let https = format!("https://{host}");
        if origin != http && origin != https {
            return Err(ApiError::new(StatusCode::FORBIDDEN, "origin mismatch"));
        }
    }
    Ok(session)
}

fn require_session(
    state: &AppState,
    headers: &HeaderMap,
) -> std::result::Result<Session, ApiError> {
    let session_id = cookie(headers, "mdboard_session")
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "not signed in"))?;
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "session lock poisoned"))?;
    let session = sessions
        .get(&session_id)
        .cloned()
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "session not found"))?;
    if session.expires < Utc::now() {
        sessions.remove(&session_id);
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "session expired"));
    }
    Ok(session)
}

fn rebuild(state: &AppState) -> std::result::Result<(), ApiError> {
    build(&state.root)
        .map(|_| ())
        .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (key, value) = cookie.trim().split_once('=')?;
                (key == name).then(|| value.to_owned())
            })
        })
}

async fn websocket(
    upgrade: WebSocketUpgrade,
    AxumState(state): AxumState<AppState>,
) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket| websocket_loop(socket, state.sender.subscribe()))
}

async fn websocket_loop(mut socket: WebSocket, mut receiver: broadcast::Receiver<()>) {
    loop {
        tokio::select! {
            message = socket.recv() => {
                if message.is_none() {
                    break;
                }
            }
            result = receiver.recv() => {
                if result.is_ok() && socket.send(Message::Text("reload".into())).await.is_err() {
                    break;
                }
            }
        }
    }
}

fn report(output: &BuildOutput) {
    println!(
        "mdboard: {} cards, {} findings, upstream `{}` → {}",
        output.cards,
        output.findings,
        output.upstream,
        output.site_dir.display()
    );
}

#[cfg(test)]
mod tests {
    use super::cookie;
    use axum::http::{HeaderMap, HeaderValue, header};
    use std::fs;
    use tempfile::TempDir;

    fn process_board() -> TempDir {
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
"#,
        )
        .unwrap();
        fs::write(
            dir.path().join(".mdboard/actors.toml"),
            r#"[actors.owner]
kind = "human"
display_name = "Owner"
roles = ["owner"]

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
        fs::write(
            dir.path().join(".mdboard/cards/0001.md"),
            "---\nid: \"#0001\"\nkind: idea\nstate: spark\ntitle: \"First idea\"\ncreated_by: owner\nintent_owner: owner\ncreated_at: 2026-10-24T10:00:00Z\nupdated_at: 2026-10-24T10:00:00Z\n---\n\n## Intent\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn cookie_parser_finds_the_session() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; mdboard_session=abc; b=2"),
        );
        assert_eq!(cookie(&headers, "mdboard_session").as_deref(), Some("abc"));
        assert_eq!(cookie(&headers, "missing"), None);
    }

    #[test]
    fn process_board_builds_static_site_and_views() {
        let dir = process_board();
        let output = super::build(dir.path()).unwrap();
        assert!(output.site_dir.join("index.html").exists());
        assert!(output.site_dir.join("api/board.json").exists());
        assert!(output.site_dir.join("cards/0001.html").exists());
        assert!(output.site_dir.join("api/cards/0001.json").exists());
        let index = fs::read_to_string(output.site_dir.join("index.html")).unwrap();
        assert!(index.contains("Board overview"));
        let api = fs::read_to_string(output.site_dir.join("api/board.json")).unwrap();
        assert!(api.contains("\"schema\": 3"));
        assert!(dir.path().join("BOARD.md").exists());
        assert!(dir.path().join("ROADMAP.md").exists());
    }
}
