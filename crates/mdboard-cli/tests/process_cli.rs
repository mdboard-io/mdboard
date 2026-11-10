use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_mdboard")
}

fn run(board: &Path, args: &[&str]) -> Output {
    Command::new(binary())
        .current_dir(board)
        .args(args)
        .output()
        .expect("run mdboard")
}

fn ok(board: &Path, args: &[&str]) -> Output {
    let output = run(board, args);
    assert!(
        output.status.success(),
        "command failed: {args:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn fail(board: &Path, args: &[&str]) -> Output {
    let output = run(board, args);
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: {args:?}"
    );
    output
}

fn git(board: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(board)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git failed: {args:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init(board: &Path) {
    ok(
        board,
        &[
            "init",
            ".",
            "--init-git",
            "--upstream",
            "main",
            "--subscription",
            "test",
            "--eur-per-million-tokens",
            "0.21",
        ],
    );
    std::fs::OpenOptions::new()
        .append(true)
        .open(board.join("mdboard.toml"))
        .unwrap()
        .write_all(b"\n[quality_gate]\nenabled = false\n")
        .unwrap();
    git(board, &["add", "-A"]);
    git(
        board,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-m",
            "init",
        ],
    );
    ok(
        board,
        &["--actor", "agent", "upstream", "start", "--base", "HEAD"],
    );
}

fn to_breakdown(board: &Path) {
    ok(
        board,
        &[
            "--actor",
            "agent",
            "refine",
            "#0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "refine",
        ],
    );
    ok(
        board,
        &[
            "--actor",
            "agent",
            "pitch",
            "#0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "pitch",
        ],
    );
    ok(
        board,
        &[
            "--actor",
            "agent",
            "accept",
            "#0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "accept",
        ],
    );
    ok(
        board,
        &[
            "--actor",
            "agent",
            "estimate",
            "#0001",
            "--tokens-optimistic",
            "1",
            "--tokens-realistic",
            "2",
            "--tokens-pessimistic",
            "3",
            "--agent-hours-optimistic",
            "1",
            "--agent-hours-realistic",
            "2",
            "--agent-hours-pessimistic",
            "3",
        ],
    );
    ok(
        board,
        &[
            "--actor",
            "agent",
            "roadmap",
            "#0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "roadmap",
        ],
    );
    ok(
        board,
        &[
            "--actor",
            "agent",
            "breakdown",
            "#0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "breakdown",
        ],
    );
    ok(
        board,
        &["--actor", "agent", "task", "new", "#0001", "Implement"],
    );
}

#[test]
fn new_board_reaches_completed_and_views_are_current() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    to_breakdown(board);
    ok(
        board,
        &[
            "--actor",
            "agent",
            "start",
            "#0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "start",
        ],
    );
    ok(board, &["--actor", "agent", "work", "#0001-01"]);
    ok(
        board,
        &[
            "--actor", "agent", "done", "#0001-01", "--commit", "deadbeef",
        ],
    );
    ok(board, &["--actor", "agent", "handover", "#0001"]);
    ok(
        board,
        &[
            "--actor",
            "owner",
            "review",
            "#0001",
            "--reviewer",
            "member",
            "--authored-by",
            "member",
            "--verdict",
            "accepted",
        ],
    );
    fail(board, &["--actor", "member", "complete", "#0001"]);
    ok(board, &["--actor", "owner", "complete", "#0001"]);
    ok(board, &["check"]);
    ok(board, &["render", "--check"]);

    let schema = ok(board, &["schema"]);
    let schema_text = String::from_utf8_lossy(&schema.stdout);
    assert!(schema_text.contains("\"schema\": 2"));
    let whoami = ok(board, &["whoami"]);
    assert!(String::from_utf8_lossy(&whoami.stdout).contains("agent"));

    let agents = std::fs::read_to_string(board.join("AGENTS.md")).unwrap();
    assert!(agents.contains("You are the **agent**"));
    assert!(!agents.contains("space new"));
    let card = std::fs::read_to_string(board.join(".mdboard/cards/0001.md")).unwrap();
    assert!(card.contains("state: completed"));
    assert!(board.join(".mdboard/reports/R0001.md").exists());
    assert!(board.join("BOARD.md").exists());
    assert!(board.join("ROADMAP.md").exists());
}

#[test]
fn start_is_refused_while_clarification_is_open() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    to_breakdown(board);
    ok(
        board,
        &[
            "--actor",
            "agent",
            "meeting",
            "open",
            "breakdown-clarification",
            "--batch",
            "#0001",
            "--trigger",
            "questions",
        ],
    );
    ok(
        board,
        &[
            "--actor",
            "agent",
            "meeting",
            "question",
            "add",
            "M0001-01",
            "Question?",
        ],
    );
    let output = fail(
        board,
        &[
            "--actor",
            "agent",
            "start",
            "#0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "start",
        ],
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("clarification"));
}

#[test]
fn card_actions_require_an_open_cycle() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    ok(
        board,
        &[
            "init",
            ".",
            "--init-git",
            "--upstream",
            "main",
            "--subscription",
            "test",
            "--eur-per-million-tokens",
            "0.21",
        ],
    );
    let output = fail(
        board,
        &[
            "--actor",
            "agent",
            "new",
            "--kind",
            "proposal",
            "One",
            "--intent-owner",
            "owner",
        ],
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("no open upstream iteration"));
}

#[test]
fn context_json_exposes_schema_three_governance() {
    let dir = TempDir::new().unwrap();
    init(dir.path());
    let output = ok(dir.path(), &["context", "--json"]);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema"], 3);
    assert_eq!(value["governance"]["max_seats"], 5);
    assert_eq!(value["governance"]["quorum_seats"], 3);
    assert_eq!(
        value["governance"]["guardian"]["email"],
        "owner@example.com"
    );
}

#[test]
fn guardian_and_stakeholder_commands_use_schema_three_config() {
    let dir = TempDir::new().unwrap();
    init(dir.path());
    let output = ok(dir.path(), &["guardian", "show"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("owner@example.com"));
    let output = ok(dir.path(), &["stakeholder", "list"]);
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}

#[test]
fn standing_order_lifecycle_is_reflected_in_runbook_and_context() {
    let dir = TempDir::new().unwrap();
    init(dir.path());
    ok(
        dir.path(),
        &[
            "--actor",
            "agent",
            "order",
            "propose",
            "--title",
            "Minor bump",
            "--instruction",
            "Bump the minor version when the roadmap is complete.",
            "--project",
            "--authorised-by",
            "owner",
            "--authorization",
            "Add standing order",
        ],
    );
    ok(
        dir.path(),
        &[
            "--actor",
            "agent",
            "order",
            "add",
            "SO-0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "Activate standing order",
        ],
    );
    let runbook = String::from_utf8_lossy(&ok(dir.path(), &["runbook"]).stdout).to_string();
    assert!(runbook.contains("Active standing orders"));
    assert!(runbook.contains("SO-0001"));
    let context = ok(dir.path(), &["context", "--json"]);
    let value: serde_json::Value = serde_json::from_slice(&context.stdout).unwrap();
    assert_eq!(value["standing_orders"]["active"], 1);
    ok(dir.path(), &["check"]);
}

#[test]
fn layout_guard_rejects_unknown_and_missing_entries_but_allows_site() {
    let dir = TempDir::new().unwrap();
    init(dir.path());
    ok(dir.path(), &["check"]);

    std::fs::create_dir_all(dir.path().join(".mdboard/topic")).unwrap();
    let output = fail(dir.path(), &["check"]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("LAYOUT-001"));
    std::fs::remove_dir_all(dir.path().join(".mdboard/topic")).unwrap();

    std::fs::create_dir_all(dir.path().join(".mdboard/site")).unwrap();
    ok(dir.path(), &["check"]);

    std::fs::remove_dir_all(dir.path().join(".mdboard/cards")).unwrap();
    let output = fail(dir.path(), &["check"]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("LAYOUT-002"));
}

#[test]
fn mirror_copies_a_clean_snapshot_and_excludes_mdboard() {
    let dir = TempDir::new().unwrap();
    init(dir.path());
    git(dir.path(), &["branch", "-m", "nightly"]);
    std::fs::write(dir.path().join("README.md"), "# Public\n").unwrap();
    let target = TempDir::new().unwrap();
    let bare = target.path().join("public.git");
    let status = Command::new("git")
        .args(["init", "-q", "--bare"])
        .arg(&bare)
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::write(
        dir.path().join(".mdboard/mirror.toml"),
        format!(
            "[profiles.public]\nto = {:?}\nbranch = \"main\"\nignore = \".mdboardignore\"\n",
            bare.display().to_string()
        ),
    )
    .unwrap();
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "mirror setup",
        ],
    );
    ok(
        dir.path(),
        &[
            "--actor",
            "agent",
            "mirror",
            "--profile",
            "public",
            "--dry-run",
        ],
    );
    ok(
        dir.path(),
        &[
            "--actor",
            "agent",
            "mirror",
            "--profile",
            "public",
            "--push",
            "--confirm-first-push",
        ],
    );
    let verify = target.path().join("verify");
    let status = Command::new("git")
        .args(["clone", "-q", "-b", "main"])
        .arg(&bare)
        .arg(&verify)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(verify.join("README.md").is_file());
    assert!(!verify.join(".mdboard").exists());
    assert!(!verify.join(".mdboardignore").exists());
}

#[test]
fn archive_and_unarchive_round_trip() {
    let dir = TempDir::new().unwrap();
    init(dir.path());
    ok(
        dir.path(),
        &[
            "--actor",
            "agent",
            "new",
            "--kind",
            "proposal",
            "Archive me",
        ],
    );
    ok(
        dir.path(),
        &[
            "--actor",
            "agent",
            "archive",
            "#0002",
            "--reason",
            "test card",
            "--authorised-by",
            "owner",
            "--authorization",
            "archive this test card",
        ],
    );
    let archived = String::from_utf8_lossy(&ok(dir.path(), &["archived"]).stdout).to_string();
    assert!(archived.contains("#0002"));
    ok(
        dir.path(),
        &[
            "--actor",
            "agent",
            "unarchive",
            "#0002",
            "--authorised-by",
            "owner",
            "--authorization",
            "restore this card",
        ],
    );
    let archived = String::from_utf8_lossy(&ok(dir.path(), &["archived"]).stdout).to_string();
    assert!(!archived.contains("#0002"));
}
