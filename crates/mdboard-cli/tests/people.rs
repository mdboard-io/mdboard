use std::io::Write;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_mdboard")
}

fn copy_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/minimal");
    fn walk(from: &Path, to: &Path) {
        for entry in std::fs::read_dir(from).unwrap().flatten() {
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                std::fs::create_dir_all(&target).unwrap();
                walk(&entry.path(), &target);
            } else {
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    for entry in std::fs::read_dir(&fixture).unwrap().flatten() {
        let target = dir.path().join(entry.file_name());
        if entry.path().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            walk(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
    dir
}
fn run(board: &Path, args: &[&str]) -> Output {
    Command::new(binary())
        .current_dir(board)
        .args(args)
        .output()
        .expect("run")
}
fn ok(board: &Path, args: &[&str]) {
    let out = run(board, args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn person_add_retire_and_preserve_comments() {
    let dir = copy_fixture();
    let actors = dir.path().join(".mdboard/actors.toml");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&actors)
        .unwrap()
        .write_all(b"\n# keep me\n")
        .unwrap();
    ok(
        dir.path(),
        &[
            "person",
            "add",
            "jane",
            "--display-name",
            "Jane",
            "--role",
            "member",
            "--git-email",
            "jane@example.com",
            "--email",
            "jane@example.com",
            "--seats",
            "1",
            "--until",
            "2027-11-02",
            "--authorised-by",
            "owner",
            "--authorization",
            "Add Jane",
        ],
    );
    let text = std::fs::read_to_string(&actors).unwrap();
    assert!(text.contains("[actors.jane]"));
    assert!(text.contains("# keep me"));
    ok(
        dir.path(),
        &[
            "actor-remove",
            "jane",
            "--retire",
            "--authorised-by",
            "owner",
            "--authorization",
            "Retire",
        ],
    );
    assert!(
        std::fs::read_to_string(&actors)
            .unwrap()
            .contains("retired = true")
    );
    let out = run(
        dir.path(),
        &["--actor", "jane", "new", "--kind", "proposal", "blocked"],
    );
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("retired"));
}
