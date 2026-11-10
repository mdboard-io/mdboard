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

fn git(board: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
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
    git(board, &["config", "user.name", "Test"]);
    git(board, &["config", "user.email", "test@example.com"]);
    let actors = std::fs::read_to_string(board.join(".mdboard/actors.toml")).unwrap();
    let actors = actors.replace(
        "[actors.owner]\nkind = \"human\"\ndisplay_name = \"Owner\"\n",
        "[actors.owner]\nkind = \"human\"\ndisplay_name = \"Owner\"\ngit_name = \"Human Owner\"\ngit_email = \"owner@example.com\"\n",
    );
    std::fs::write(board.join(".mdboard/actors.toml"), actors).unwrap();
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
    ok(board, &["--actor", "agent", "upstream", "start"]);
}

#[test]
fn human_semantic_author_is_separate_from_agent_committer() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    std::fs::write(board.join("decision.txt"), "approved\n").unwrap();
    ok(
        board,
        &[
            "--actor",
            "agent",
            "commit",
            "--as",
            "owner",
            "--authorised-by",
            "owner",
            "--authorization",
            "Approve the decision",
            "-m",
            "record decision",
        ],
    );
    let identity = git(board, &["log", "-1", "--format=%an|%ae|%cn|%ce"]);
    assert_eq!(
        identity,
        "Human Owner|owner@example.com|agent|no-replay@example.com"
    );
    let message = git(board, &["log", "-1", "--format=%B"]);
    assert!(message.contains("Authorised-by: owner"), "{message}");
    assert!(
        message.contains("Authorization: Approve the decision"),
        "{message}"
    );
    assert!(message.contains("Performed-by: agent"), "{message}");
}

#[test]
fn human_semantic_author_requires_authorisation() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    std::fs::write(board.join("decision.txt"), "approved\n").unwrap();
    let output = run(
        board,
        &[
            "--actor",
            "agent",
            "commit",
            "--as",
            "owner",
            "-m",
            "record decision",
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("requires --authorised-by"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(git(board, &["status", "--porcelain"]).contains("decision.txt"));
}

#[test]
fn mdboard_identity_commits_generated_output() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    std::fs::write(board.join("BOARD.md"), "generated\n").unwrap();
    ok(
        board,
        &[
            "--actor",
            "agent",
            "commit",
            "--identity",
            "mdboard",
            "-m",
            "render board",
        ],
    );
    let identity = git(board, &["log", "-1", "--format=%an|%ae|%cn|%ce"]);
    assert_eq!(
        identity,
        "mdboard|no-reply@mdboard.io|mdboard|no-reply@mdboard.io"
    );
    let message = git(board, &["log", "-1", "--format=%B"]);
    assert!(message.contains("Generated-by: mdboard"), "{message}");
}

#[test]
fn agent_identity_rejects_generated_output() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    std::fs::write(board.join("BOARD.md"), "generated\n").unwrap();
    let output = run(
        board,
        &[
            "--actor",
            "agent",
            "commit",
            "--identity",
            "agent",
            "-m",
            "wrong class",
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("generated output"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn mdboard_identity_rejects_source_code() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    std::fs::create_dir_all(board.join("crates/demo/src")).unwrap();
    std::fs::write(board.join("crates/demo/src/lib.rs"), "pub fn demo() {}\n").unwrap();
    let output = run(
        board,
        &[
            "--actor",
            "agent",
            "commit",
            "--identity",
            "mdboard",
            "-m",
            "wrong class",
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot commit source code"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
