# baton

`baton` is a thin CLI that starts and drives agents (Pi, Claude Code) in terminal multiplexer panes. Lead and Conductor skills call `baton`, not the multiplexer, so the multiplexer can change later. v1 has one adapter: Zellij 0.46.1.

Scope: mechanics only. `baton` does not create jj workspaces, read Beads or decide what to do next.

- Spec (authority): [`docs/design.md`](docs/design.md)
- Agent skill: [`skills/using-baton/SKILL.md`](skills/using-baton/SKILL.md)
- Tasks: Beads in the agent-workflows store, `bd -C ~/projects/agent-workflows …` (bead `aw-i96` built v1).

## Install from a release (no build tools)

Each `v*` tag builds ready-to-use archives on GitHub Releases. Each archive holds the binary, `skills/using-baton` and this README.

- **Windows (PowerShell):** download `baton-<version>-x86_64-pc-windows-msvc.zip` (browser, or `gh release download -R MrAldrin/baton -p "*windows*"`). Unzip, put `baton.exe` in `$HOME\.local\bin` and add that folder to your user PATH. Copy or junction `skills\using-baton` to `$HOME\.claude\skills\using-baton` and `$HOME\.pi\agent\skills\using-baton`.
- **Linux and WSL:** download `baton-<version>-x86_64-unknown-linux-musl.tar.gz` (static, works on any x86_64 distribution). Unpack, put `baton` in `~/.local/bin`, and link `skills/using-baton` into `~/.claude/skills/` and `~/.pi/agent/skills/`.
- Check: `baton --version`.

To make a release: `jj tag set vX.Y.Z -r main && jj git push --tag vX.Y.Z` (or `git push origin vX.Y.Z`). Bump `version` in `Cargo.toml` first.

## Quick start

```bash
cargo build --release
S=my-session
baton --session $S run --name worker --cwd ~/scratch --tab-id 0 --agent pi
baton --session $S send terminal_3 "Say hello"
baton --session $S wait terminal_3 --timeout 300
baton --session $S read terminal_3 --lines 20
baton --session $S close terminal_3
```

Inside Zellij, `--session` defaults to `$ZELLIJ_SESSION_NAME`, and `run` opens a floating pane in your own tab.

## Verbs

| Verb | Does |
|---|---|
| `run --name N --cwd D [--tab-id T \| --new-tab NAME] [--agent pi\|claude] -- cmd…` | Start a program without focus; record ownership. Prints `{pane, tab_id, session_id?}`. |
| `send <pane> <text> [--no-enter] [--force]` | Type text and press Enter. Refuses `working` or `blocked` panes. |
| `keys <pane> <key…> [--force]` | `enter`, `esc`, `tab`, `up`, `down`, `ctrl-c`, digits. |
| `read <pane> [--full] [--lines N] [--json]` | The pane's text. |
| `status <pane>` | `{state, source, app, message, updated_ms, exit_code}`. |
| `wait <pane> [--until done,blocked,error,exited] [--timeout S]` | Block until a state is reached after the last send. |
| `list [--owned]` | Terminal panes with name, title, command, cwd and state. For a pane `baton` owns, `name` is the `run --name`; `title` is the live pane title, which agents change. |
| `close <pane> [--force]` | Close a pane `baton` started. |

`--force` is needed to write to or close a pane that `baton` did not start.

`wait` ignores a `done`, `idle`, `error` or `blocked` state reported before the last `send` or `keys`. A forced `send` or `keys` to a pane `baton` did not start writes a minimal record (`owned: false`) that only keeps the send time; the pane stays not owned.

Output is one JSON object on stdout (`read` prints text unless `--json`). Errors print `{"error": {"message": …}}`.

Exit codes: `0` done or idle, `1` error or exited, `2` blocked, `124` timeout, `3` other error.

Records: `$XDG_STATE_HOME/baton/<session>~<pane>.json` (default `~/.local/state/baton/`).

## Agents

- `--agent pi|claude` adds `--session-id <uuid>`, so a pane can be resumed later (`pi --session <id>`, `claude --resume <id>`). For a script runner such as `node launch.mjs`, the flag goes after the script.
- Pi and Claude Code report their state themselves (OSC 7501).
- On Windows, use Zellij 0.46.1: with 0.46.0 the answer to the agents' OSC 7501 feature query is lost, and Claude Code reports no state. `run --agent pi` also sets `PI_PROGRAM_STATUS=1` (unless set), so Pi reports even then.
- `run` starts every program through `env -u …` without the variables a Claude Code host sets for its children (list: `CLAUDE_HOST_VARS` in `src/commands.rs`). Inherited, they make Claude Code a child session that sends no state (`CLAUDE_CODE_DISABLE_TERMINAL_TITLE`). Auth and account variables are kept.
- A new, untrusted folder makes Claude Code ask for trust first. That prompt shows as `unknown`, not `blocked`.

## Repository map

| Path | Purpose |
|---|---|
| `src/host.rs` | The `Host` trait: the adapter boundary. |
| `src/zellij.rs` | Zellij adapter and state mapping. |
| `src/commands.rs` | The verbs. |
| `src/state.rs` | States, the stale-`Done` rule, `wait` exit codes. |
| `src/store.rs` | Ownership records. |
| `src/agent.rs` | `--agent` session ids. |
| `tests/cli.rs` | Integration tests on throwaway Zellij sessions. |
| `tests/fixtures/` | `list-panes` JSON and the fake OSC 7501 agent. |
