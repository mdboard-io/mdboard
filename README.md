# mdBoard

**Human intent, agent execution, one board.**

mdBoard is a collaboration protocol for a human team and a coding agent. You talk to your
agent; the agent operates mdBoard; mdBoard keeps intent, review gates and Git-backed
decisions aligned.

It is not an issue tracker, an agent runtime or a chat workspace. It is the small, strict layer
that keeps an agent's speed from outrunning human understanding.

## The model

- **Humans** set intent, accept ideas, answer clarification, review batches, merge and release.
- **The agent** refines, estimates, breaks down, implements, commits and reports.
- **mdBoard** records states and events, enforces gates, coordinates meetings and generates views.

There is one active `upstream` iteration, one agent writer lease and one active batch. Review
happens per batch at `handover`, not per task.

```text
spark -> refine -> pitch -> accepted -> roadmap -> breakdown -> doing -> handover -> completed
```

## Build and install from source

```sh
cargo build --release -p mdboard-cli
install -m 755 target/release/mdboard "$HOME/.local/bin/mdboard"
export PATH="$HOME/.local/bin:$PATH"
mdboard --version
```

Or install with Cargo:

```sh
cargo install --path crates/mdboard-cli --locked --root "$HOME/.local"
```

The repository board is initialized in `.mdboard/`. Before the first iteration, commit the
initialization and open it:

```sh
git add -A
git commit -m "Initialize mdBoard process board"
mdboard --actor agent upstream start
mdboard context
```

## Quick start

```sh
mdboard init
# commit the initialized board, then connect your agent
mdboard --actor agent upstream start
mdboard --actor agent spark "CSV export for the report table"
mdboard context
mdboard serve --actor owner
```

The agent reads `AGENTS.md`, then runs `mdboard runbook`. The runbook is normative.

Typical human gates:

- accept or reject a pitch at the consortium;
- choose and estimate a roadmap batch;
- answer breakdown clarification;
- accept a batch review;
- merge `upstream -> nightly` and release `nightly -> stable`.

Only the owner merges or releases. The agent prepares checks, reports and release notes.

## After initialization: let the agent look up costs

The default cost model is a planning placeholder. After `mdboard init`, you can ask the agent to look up current pricing for the model and subscription you actually use:

```text
Human: Look up the current DeepSeek-V4.1-Flash pricing for our off-peak usage
       and adapt the cost estimates.
Agent: I checked the provider pricing on 2026-10-25. For this subscription
          the blended rate is €0.21 per 1M tokens at our 80/20 input/output mix.
          I will update [costs] and refresh the roadmap if you approve.
```

The agent cites the source and date, states the assumptions and updates `[costs]` only with owner approval. mdBoard itself does not fetch prices. See [estimation and cost](https://mdboard.io/guide/estimation).

## Commands

```sh
mdboard runbook
mdboard context
mdboard actions '#0007'
mdboard attachment list '#0007'
mdboard attachment path '#0007' SPEC.md
mdboard brief
mdboard history '#0007'
mdboard commit -m "message"
mdboard check --git
mdboard render --check
mdboard serve --actor owner
mdboard watch
```

Named actions cover the lifecycle: `spark`, `refine`, `pitch`, `accept-idea`, `estimate`,
`roadmap`, `breakdown`, `start`, `task new`, `work`, `done`, `handover`, `review`,
`complete`, `block`, `cancel`, `meeting`, `upstream`.

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/mdboard-core` | process entities, transitions, estimates, actors |
| `crates/mdboard-driver` | process board loading and records |
| `crates/mdboard-actions` | named actions, authority, guards, events |
| `crates/mdboard-git` | Git state and branch operations |
| `crates/mdboard-renderer` | `BOARD.md` and `ROADMAP.md` |
| `crates/mdboard-serve` | HTML board and JSON API |
| `crates/mdboard-cli` | agent and owner CLI |
| `.mdboard/` | mdBoard board: cards, events, meetings, reviews, reports, attachments |
| `BOARD.md`, `ROADMAP.md` | generated board views |
| `docs/spec` | formal process model |

## Development gate

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
git diff --check
```

## License

GNU Affero General Public License v3.0 only. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
