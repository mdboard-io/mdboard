# Contributing to mdBoard

mdBoard is designed for **human control and agent execution**. Contributions are welcome, but
[AGENTS.md](AGENTS.md) and the [process model](docs/spec/process-model.md) are normative.

## Before you start

- Read [README.md](README.md), [AGENTS.md](AGENTS.md) and the relevant process model section.
- Search existing issues and discussions.
- For a substantial change, start with a spark and shape it through refine and pitch.
- Security issues go through [SECURITY.md](SECURITY.md), not a public issue.

## Development setup

```sh
git clone https://github.com/mdboard-io/mdboard.git
cd mdboard
cargo build --release -p mdboard-cli
cargo test --workspace
```

Requirements: Rust 1.88, Git and a Unix-like shell.

## Required checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

tmp="$(mktemp -d)"
cp -R tests/fixtures/minimal "$tmp/board"
cargo run -p mdboard-cli -- --root "$tmp/board" check
cargo run -p mdboard-cli -- --root "$tmp/board" render
cargo run -p mdboard-cli -- --root "$tmp/board" render --check
rm -rf "$tmp"
```

CI runs the same gate on Linux and macOS. A pull request also builds the Docker image without
pushing it. Publishing is tag-only and requires a passing `verify` job first.

## Board workflow

- The repository board is in `.mdboard/`; `BOARD.md` and `ROADMAP.md` are generated views.
- The agent operates the board with named actions; humans make the gates.
- One active batch, one task working and one writer lease.
- Review is per batch at `handover`.
- The owner executes `upstream merge` and `release`; the agent prepares checks and reports.
- Commit task and batch boundaries with `mdboard commit -m "…"` and leave the tree clean for the
  owner.
- Never hand-edit cards, events, reports or generated views.

## Code guidelines

- Keep the core free of Git, HTTP, HTML and terminal dependencies.
- Put shared mutations in `mdboard-actions`; every surface must use them.
- Keep views generated; never hand-edit output.
- Add a test for behavior that can regress.
- Do not add network, telemetry or analytics without an owner decision.
- Do not claim a sandbox the code does not enforce.

## Documentation

- The VitePress source is in `web/mdboard-io`; build it with `npm run docs:build` from that directory.
- Keep commands and examples runnable.
- Update the relevant reference when a flag, field or endpoint changes.
- Do not include private operator data, customer names, tokens or secrets.

## Commits and pull requests

- One logical change per commit.
- Explain the why, the behavior change and the checks run.
- Reference the card ID when the change was executed under the batch.
- Do not rewrite history after handover.
- Do not merge into `nightly` or `stable` unless you are the owner.
- Agents propose and execute inside a batch; humans review and merge.

## License

By contributing, you agree that your contribution is licensed under the GNU Affero General Public License v3.0 only. There
is no separate CLA.
