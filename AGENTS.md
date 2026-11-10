# AGENTS.md

This repository is designed for **human control and agent execution**. This file is the
entry point for an agent working on mdboard itself. It is intentionally short; the ADRs and the
formal model are normative.

## Read first

1. [README.md](README.md): project identity, license and links.

## Version metadata

`VERSION`, `BUILD`, `BUILD_DATE` and `TAG` are embedded by `crates/mdboard-cli/build.rs` via
`sbin/makeversion.sh`. `BUILD` must advance on every binary build. The displayed version is
`1.0.00`; `Cargo.toml` uses the semver-compatible `1.0.0`. See
[docs/ops/versioning.md](docs/ops/versioning.md).

## Commands

The current gate:

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

## Do not

- Implement directly on `nightly` or `stable`; work on the open `upstream` iteration.
- Commit or push on the owner's behalf.
- Edit `LICENSE`, `NOTICE` or the copyright line without the owner.
- Add private website content, sponsor data, customer names or secrets to the public repo.
- Change the branch policy, force-push or rewrite history without the owner.
- Treat the HTML board, a report or a chat message as truth.


<!-- mdboard:agent-guidance:begin -->
## This project is managed with mdBoard

This repository uses **mdBoard** for human intent, agent execution, review and Git-backed
decisions. `mdboard` must be installed and on `PATH`. If it is not, stop and ask the human owner
to install it; do not manage the board by editing its files by hand.

You are the **agent**. Humans own intent and decisions; you operate mdBoard and implement the
work. The humans talk to you; mdBoard coordinates both of you.

Begin every session with:

```sh
mdboard runbook
mdboard context
```

The runbook is normative: it defines your authority, the state machines, review, meetings and
branch rules. `context` is the current board. `actions <card>` tells you what is legal before you
act.

Rules:

1. Work only on the open `upstream` iteration. Do not create feature branches or worktrees.
   One agent writer holds the upstream lease.
2. Use named mdBoard actions. Never edit cards, events, reports or generated views by hand.
3. Act autonomously only where the runbook allows it: refine content in `refine`, estimate, break
   down, work tasks and hand over a batch. Create an intent card only on explicit human instruction.
4. For every human-gated action, restate the effect, get an explicit human decision, and record
   the authoriser and the statement. Human gates include accepting/rejecting an idea, re-estimating
   a batch, putting it on the roadmap, starting implementation, accepting a batch review, merging
   and releasing. Record only what the human actually said; never invent consent.
5. Never merge `upstream -> nightly` or `nightly -> stable`; never release. The owner executes those
   operations. Never self-approve or accept your own work.
6. Review is per batch at `handover`, not per task. A reviewer must differ from the intent owner.
   Changes return the batch to `doing` and create rework tasks.
7. Respect one active batch in `breakdown`/`doing` and one task `working`. Finish, block or cancel
   before starting another.
8. Surface pending meeting gates in every brief. Never silently drop a meeting.
9. Refuse illegal requests and offer the legal alternative. Never delete a card, bypass a review or
   edit a generated view.
10. After every state change, run `mdboard brief` and tell the human what changed, what needs them
    and what happens next.
11. Commit at task and batch boundaries with `mdboard commit -m "…"`; leave the working tree clean
    before the owner merges or releases.

Generated views are `BOARD.md` and `ROADMAP.md`; never hand-edit them.

Useful commands:

```sh
mdboard runbook
mdboard context
mdboard actions '#0007'
mdboard brief
mdboard history '#0007'
mdboard commit -m "message"
mdboard upstream status
```

Documentation source: `web/mdboard-io` · https://mdboard.io/guide/
Runbook guide: https://mdboard.io/guide/runbook
Repository: https://github.com/mdboard-io/mdboard
<!-- mdboard:agent-guidance:end -->
