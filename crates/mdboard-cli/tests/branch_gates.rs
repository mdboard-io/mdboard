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

fn mdboard(board: &Path, args: &[&str]) -> Output {
    let output = run(board, args);
    assert!(
        output.status.success(),
        "mdboard failed: {args:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn fails(board: &Path, args: &[&str]) -> Output {
    let output = run(board, args);
    assert!(!output.status.success(), "unexpected success: {args:?}");
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

fn commit(board: &Path, message: &str) {
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
            message,
        ],
    );
}

fn init(board: &Path) {
    mdboard(
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
    std::fs::OpenOptions::new()
        .append(true)
        .open(board.join("mdboard.toml"))
        .unwrap()
        .write_all(b"\n[quality_gate]\nenabled = false\n")
        .unwrap();
    commit(board, "init");
    git(board, &["branch", "nightly"]);
    mdboard(board, &["--actor", "agent", "upstream", "start"]);
    commit(board, "open iteration");
}

fn to_doing(board: &Path) {
    mdboard(
        board,
        &[
            "--actor",
            "agent",
            "refine",
            "#0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "r",
        ],
    );
    mdboard(
        board,
        &[
            "--actor",
            "agent",
            "pitch",
            "#0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "p",
        ],
    );
    mdboard(
        board,
        &[
            "--actor",
            "agent",
            "accept",
            "#0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "a",
        ],
    );
    mdboard(
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
    mdboard(
        board,
        &[
            "--actor",
            "agent",
            "roadmap",
            "#0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "r",
        ],
    );
    mdboard(
        board,
        &[
            "--actor",
            "agent",
            "breakdown",
            "#0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "b",
        ],
    );
    mdboard(board, &["--actor", "agent", "task", "new", "#0001", "Task"]);
    mdboard(
        board,
        &[
            "--actor",
            "agent",
            "start",
            "#0001",
            "--authorised-by",
            "owner",
            "--authorization",
            "s",
        ],
    );
}

#[test]
fn owner_can_merge_then_release_a_clean_cycle() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    git(board, &["branch", "stable"]);
    mdboard(board, &["--actor", "owner", "upstream", "merge"]);
    let record = std::fs::read_to_string(board.join(".mdboard/upstream.md")).unwrap();
    assert!(record.contains("status: merged"));
    assert!(git(board, &["log", "--format=%s", "nightly"]).contains("Merge upstream iteration 1"));
    assert_eq!(git(board, &["status", "--porcelain"]), "");
    mdboard(board, &["--actor", "owner", "upstream", "release"]);
    let version = include_str!("../../../VERSION").trim();
    let tag = format!("v{version}");
    assert!(git(board, &["tag", "--list", &tag]).contains(&tag));
    assert!(git(board, &["log", "-1", "--format=%s", "stable"]).contains("Release"));
    assert!(
        git(
            board,
            &["show", &format!("stable:.mdboard/releases/{tag}.md")]
        )
        .contains("Release")
    );
}

#[test]
fn merge_is_owner_only() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    let output = fails(board, &["--actor", "member", "upstream", "merge"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("required role"));
}

#[test]
fn start_refuses_a_dirty_worktree() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    mdboard(
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
    commit(board, "init");
    std::fs::write(board.join("dirty.txt"), "dirty").unwrap();
    let output = fails(board, &["--actor", "agent", "upstream", "start"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("dirty"));
}

#[test]
fn git_check_reports_an_expired_lease() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    let mut record = std::fs::read_to_string(board.join(".mdboard/upstream.md")).unwrap();
    record = record.replace(
        record
            .lines()
            .find(|line| line.starts_with("lease_expires:"))
            .unwrap(),
        "lease_expires: 2000-01-01T00:00:00Z",
    );
    std::fs::write(board.join(".mdboard/upstream.md"), record).unwrap();
    let output = fails(board, &["check", "--git"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("UPSTREAM-001"));
}

#[test]
fn cleared_lease_blocks_the_agent() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    mdboard(board, &["--actor", "owner", "upstream", "clear"]);
    let output = fails(
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
    assert!(String::from_utf8_lossy(&output.stderr).contains("lease"));
}

#[test]
fn agent_can_renew_its_lease() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    let output = mdboard(
        board,
        &["--actor", "agent", "upstream", "renew", "--hours", "1"],
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("lease renewed"));
}

#[test]
fn merge_refuses_an_active_batch() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    to_doing(board);
    let output = fails(board, &["--actor", "owner", "upstream", "merge"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("still in breakdown"));
}

#[test]
fn commit_command_is_lease_checked_and_records_the_sha() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    std::fs::write(board.join("work.txt"), "code").unwrap();
    let output = mdboard(board, &["--actor", "agent", "commit", "-m", "implement"]);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("committed"));
    assert!(git(board, &["status", "--porcelain"]).is_empty());
    mdboard(board, &["--actor", "owner", "upstream", "clear"]);
    let output = fails(board, &["--actor", "agent", "commit", "-m", "again"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("lease"));
}

#[test]
fn merge_conflict_is_aborted_and_reported() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    std::fs::write(board.join("AGENTS.md"), "upstream version\n").unwrap();
    commit(board, "upstream change");
    git(board, &["switch", "nightly"]);
    std::fs::write(board.join("AGENTS.md"), "nightly version\n").unwrap();
    commit(board, "nightly change");
    git(board, &["switch", "main"]);
    let output = fails(board, &["--actor", "owner", "upstream", "merge"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("conflict"));
    let reports = std::fs::read_dir(board.join(".mdboard/reports"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("conflict-"))
        .count();
    assert_eq!(reports, 1);
    assert!(!git(board, &["status", "--porcelain"]).contains("UU"));
}

#[test]
fn git_check_flags_owner_only_changes() {
    let dir = TempDir::new().unwrap();
    let board = dir.path();
    init(board);
    let mut config = std::fs::read_to_string(board.join("mdboard.toml")).unwrap();
    config.push_str("\n# owner-only change\n");
    std::fs::write(board.join("mdboard.toml"), config).unwrap();
    let output = fails(board, &["check", "--git"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("OWNER-001"));
}
