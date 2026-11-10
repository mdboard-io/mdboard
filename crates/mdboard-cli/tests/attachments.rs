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

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn copy_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/minimal");
    for entry in walk(&fixture) {
        let rel = entry.strip_prefix(&fixture).unwrap();
        let target = dir.path().join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).unwrap();
        } else {
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::copy(&entry, &target).unwrap();
        }
    }
    dir
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            paths.extend(walk(&path));
        }
        paths.push(path);
    }
    paths
}

#[test]
fn lists_and_resolves_card_attachments() {
    let dir = copy_fixture();
    let attachments = dir.path().join(".mdboard/attachments/0001");
    std::fs::create_dir_all(&attachments).unwrap();
    std::fs::write(attachments.join("notes.txt"), "design notes\n").unwrap();

    let output = run(dir.path(), &["attachment", "list", "#0001"]);
    assert!(output.status.success());
    assert_eq!(stdout(&output), ".mdboard/attachments/0001/notes.txt\n");

    let output = run(dir.path(), &["attachment", "path", "#0001", "notes.txt"]);
    assert!(output.status.success());
    assert_eq!(stdout(&output), ".mdboard/attachments/0001/notes.txt\n");

    let output = run(dir.path(), &["attachment", "path", "#0001", "../escape"]);
    assert!(!output.status.success());
}

#[test]
fn rejects_orphans_and_missing_references() {
    let dir = copy_fixture();
    let orphan = dir.path().join(".mdboard/attachments/9999");
    std::fs::create_dir_all(&orphan).unwrap();

    let output = run(dir.path(), &["check"]);
    assert!(!output.status.success());
    assert!(stdout(&output).contains("ATTACH-004"));

    std::fs::remove_dir_all(&orphan).unwrap();
    let card = dir.path().join(".mdboard/cards/0001.md");
    let mut text = std::fs::read_to_string(&card).unwrap();
    text.push_str("\n.mdboard/attachments/0001/missing.md\n");
    std::fs::write(&card, text).unwrap();

    let output = run(dir.path(), &["check"]);
    assert!(!output.status.success());
    assert!(stdout(&output).contains("ATTACH-003"));
}
