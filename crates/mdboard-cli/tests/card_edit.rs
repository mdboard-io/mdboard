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
    Command::new("git")
        .current_dir(board)
        .args(["config", "user.name", "Test"])
        .status()
        .unwrap();
    Command::new("git")
        .current_dir(board)
        .args(["config", "user.email", "test@example.com"])
        .status()
        .unwrap();
    Command::new("git")
        .current_dir(board)
        .args(["add", "-A"])
        .status()
        .unwrap();
    Command::new("git")
        .current_dir(board)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "init",
        ])
        .status()
        .unwrap();
    ok(board, &["--actor", "agent", "upstream", "start"]);
    ok(
        board,
        &["--actor", "agent", "new", "--kind", "proposal", "Original"],
    );
}

#[test]
fn edits_title_and_body_in_draft() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    std::fs::write(board.join("body.md"), "New body\n").unwrap();
    ok(
        board,
        &[
            "--actor",
            "agent",
            "edit",
            "#0001",
            "--title",
            "Revised",
            "--body-file",
            "body.md",
            "--authorised-by",
            "owner",
            "--authorization",
            "Fix the card wording",
        ],
    );
    let text = std::fs::read_to_string(board.join(".mdboard/cards/0001.md")).unwrap();
    assert!(text.contains("title: Revised"), "{text}");
    assert!(text.contains("New body"), "{text}");
    let history = ok(board, &["history", "#0001"]);
    let history = String::from_utf8_lossy(&history.stdout);
    assert!(history.contains("edit"), "{history}");
}

#[test]
fn rejects_edit_after_pitch() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    ok(
        board,
        &[
            "--actor",
            "owner",
            "refine",
            "#0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "Refine it",
        ],
    );
    ok(
        board,
        &["--actor", "agent", "quality-gate", "#0001", "--passed"],
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
            "Pitch it",
        ],
    );
    let output = run(
        board,
        &[
            "--actor",
            "agent",
            "edit",
            "#0001",
            "--title",
            "Too late",
            "--authorised-by",
            "owner",
            "--authorization",
            "Edit it",
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("only allowed in draft or refine"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn agent_edit_requires_authorisation() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    let output = run(
        board,
        &["--actor", "agent", "edit", "#0001", "--title", "Nope"],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("authoriser is required"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn rejects_body_file_with_frontmatter() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    let before = std::fs::read_to_string(board.join(".mdboard/cards/0001.md")).unwrap();
    std::fs::write(
        board.join("full-card.md"),
        "---\nid: '#0001'\nkind: proposal\n---\n\nNew body\n",
    )
    .unwrap();
    let output = run(
        board,
        &[
            "--actor",
            "agent",
            "edit",
            "#0001",
            "--body-file",
            "full-card.md",
            "--authorised-by",
            "owner",
            "--authorization",
            "Try a full card file",
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("remove the YAML frontmatter"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after = std::fs::read_to_string(board.join(".mdboard/cards/0001.md")).unwrap();
    assert_eq!(before, after);
}

#[test]
fn visibility_can_be_changed_by_owner() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    ok(
        board,
        &[
            "--actor",
            "owner",
            "visibility",
            "set",
            "#0001",
            "private",
            "--authorised-by",
            "owner",
            "--authorization",
            "Commercial card",
        ],
    );
    let text = std::fs::read_to_string(board.join(".mdboard/cards/0001.md")).unwrap();
    assert!(text.contains("visibility: private"), "{text}");
}

#[test]
fn stale_edit_is_rejected() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    let text = std::fs::read_to_string(board.join(".mdboard/cards/0001.md")).unwrap();
    let stamp = text
        .lines()
        .find(|line| line.starts_with("updated_at:"))
        .unwrap()
        .trim_start_matches("updated_at: ")
        .to_owned();
    std::thread::sleep(std::time::Duration::from_secs(1));

    ok(
        board,
        &[
            "--actor",
            "agent",
            "edit",
            "#0001",
            "--title",
            "First",
            "--authorised-by",
            "owner",
            "--authorization",
            "First edit",
            "--expect-updated-at",
            &stamp,
        ],
    );
    let after_first = std::fs::read_to_string(board.join(".mdboard/cards/0001.md")).unwrap();
    let output = run(
        board,
        &[
            "--actor",
            "agent",
            "edit",
            "#0001",
            "--title",
            "Stale",
            "--authorised-by",
            "owner",
            "--authorization",
            "Stale edit",
            "--expect-updated-at",
            &stamp,
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("changed since the edit was prepared"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        after_first,
        std::fs::read_to_string(board.join(".mdboard/cards/0001.md")).unwrap()
    );
}
