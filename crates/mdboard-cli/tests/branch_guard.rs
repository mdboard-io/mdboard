use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_mdboard")
}
fn run(board: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .current_dir(board)
        .args(args)
        .output()
        .expect("run mdboard")
}
fn ok(board: &Path, args: &[&str]) -> Output {
    let out = run(board, args);
    assert!(
        out.status.success(),
        "{args:?}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}
fn git(board: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .current_dir(board)
            .args(args)
            .status()
            .unwrap()
            .success()
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
    git(board, &["config", "user.name", "Test"]);
    git(board, &["config", "user.email", "test@example.com"]);
    git(board, &["add", "-A"]);
    git(board, &["commit", "-qm", "init"]);
    ok(board, &["--actor", "agent", "upstream", "start"]);
    git(board, &["add", "-A"]);
    git(board, &["commit", "-qm", "open"]);
}
#[test]
fn blocks_agent_writes_on_nightly_and_allows_owner_switch() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    git(board, &["switch", "-c", "nightly"]);
    let blocked = run(
        board,
        &["--actor", "agent", "new", "--kind", "proposal", "Nope"],
    );
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("implementation must run on `main`"));
    ok(board, &["--actor", "agent", "status"]);
    ok(
        board,
        &[
            "--actor",
            "agent",
            "branch",
            "switch",
            "--authorised-by",
            "owner",
            "--authorization",
            "switch",
        ],
    );
    let branch = Command::new("git")
        .current_dir(board)
        .args(["branch", "--show-current"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&branch.stdout).trim(), "main");
}
