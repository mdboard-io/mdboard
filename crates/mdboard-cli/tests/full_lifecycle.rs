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

#[test]
fn init_to_release_is_one_coherent_cycle() {
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
    git(board, &["branch", "stable"]);
    mdboard(board, &["--actor", "agent", "upstream", "start"]);

    mdboard(
        board,
        &[
            "--actor",
            "agent",
            "new",
            "--kind",
            "proposal",
            "CSV export",
            "--intent-owner",
            "owner",
        ],
    );
    for (action, auth) in [
        ("refine", "refine the idea"),
        ("pitch", "pitch it"),
        ("accept", "accept it"),
    ] {
        mdboard(
            board,
            &[
                "--actor",
                "agent",
                action,
                "#0001",
                "--authorised-by",
                "owner",
                "--authorization",
                auth,
            ],
        );
    }
    mdboard(
        board,
        &[
            "--actor",
            "agent",
            "estimate",
            "#0001",
            "--tokens-optimistic",
            "8000",
            "--tokens-realistic",
            "12000",
            "--tokens-pessimistic",
            "20000",
            "--agent-hours-optimistic",
            "0.5",
            "--agent-hours-realistic",
            "1.0",
            "--agent-hours-pessimistic",
            "1.5",
        ],
    );
    for (action, auth) in [("roadmap", "roadmap it"), ("breakdown", "break it down")] {
        mdboard(
            board,
            &[
                "--actor",
                "agent",
                action,
                "#0001",
                "--authorised-by",
                "owner",
                "--authorization",
                auth,
            ],
        );
    }
    mdboard(
        board,
        &[
            "--actor",
            "agent",
            "task",
            "new",
            "#0001",
            "Implement export",
        ],
    );
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
            "start it",
        ],
    );
    mdboard(board, &["--actor", "agent", "work", "#0001-01"]);
    std::fs::write(board.join("export.txt"), "implemented\n").unwrap();
    let output = mdboard(
        board,
        &[
            "--actor",
            "agent",
            "commit",
            "-m",
            "#0001-01 implement export",
        ],
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("committed"));
    let sha = git(board, &["rev-parse", "HEAD"]);
    mdboard(
        board,
        &["--actor", "agent", "done", "#0001-01", "--commit", &sha],
    );
    mdboard(board, &["--actor", "agent", "handover", "#0001"]);
    mdboard(
        board,
        &["--actor", "agent", "commit", "-m", "#0001 handover"],
    );
    mdboard(
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
    mdboard(board, &["--actor", "agent", "commit", "-m", "#0001 review"]);
    mdboard(board, &["--actor", "owner", "complete", "#0001"]);
    mdboard(
        board,
        &[
            "--actor",
            "agent",
            "commit",
            "-m",
            "#0001 complete and report",
        ],
    );

    mdboard(board, &["check", "--git"]);
    mdboard(board, &["render", "--check"]);
    mdboard(board, &["--actor", "owner", "upstream", "merge"]);
    assert_eq!(git(board, &["status", "--porcelain"]), "");
    mdboard(board, &["--actor", "owner", "upstream", "release"]);
    let version = include_str!("../../../VERSION").trim();
    let tag = format!("v{version}");
    assert_eq!(git(board, &["tag", "--list", &tag]), tag);
    assert!(
        git(
            board,
            &["show", &format!("stable:.mdboard/releases/{tag}.md")]
        )
        .contains("Completed batches")
    );
    let board_md = std::fs::read_to_string(board.join("BOARD.md")).unwrap();
    assert!(board_md.contains("completed"));
    let agents = std::fs::read_to_string(board.join("AGENTS.md")).unwrap();
    assert!(!agents.contains("space new"));
    assert_eq!(git(board, &["status", "--porcelain"]), "");
}
