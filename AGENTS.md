# Agent instructions

## Startup

- Small Rust CLI. One binary, one adapter (Zellij 0.46.1).
- The spec is the authority: `~/projects/agent-workflows/docs/primitives/session-host/host-cli-design.md`. If code and spec differ, report it; do not guess.
- Tasks live in Beads in the agent-workflows store. Always use `bd -C ~/projects/agent-workflows …`. A direct request selects work; a bead is not permission by itself.
- `README.md` owns the repo map, verbs and quick start.

## Working here

- Version control: jj. Commit titles end with the bead id, for example `(aw-i96)`.
- Keep multiplexer calls in an adapter behind `host::Host`. Verbs never call `zellij` directly.
- Zellij exits 0 for an unknown session or pane. Check that a session or pane exists before acting on it.
- `baton hook` runs inside agent hooks. It must exit 0 and print nothing: a failing hook blocks Claude's prompt, and hook stdout can be read as a decision.
- Stack: clap derive, serde, anyhow, no async runtime. Tests: cargo-nextest, assert_cmd, insta.
- Distinguish planned, implemented and verified.

## Safeguards

- Never touch the owner's Zellij sessions. Tests and manual checks use throwaway sessions (`baton-test-…`) and delete them afterwards.
- Never focus a pane. Never write to or close a pane `baton` did not start, unless `--force`.
- `keys` never picks an "always" or "don't ask again" option; that choice stays with a person.
- Do not install `baton` into `~/.local/bin`, change skills, create a remote or push without the owner's approval.

## Checks

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo nextest run
cargo test --doc
```

Integration tests need `zellij` 0.46.1 on `PATH`.
