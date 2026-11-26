# Host CLI (`baton`): v1 design

Beads: design `aw-gwk`, build `aw-i96`. Approved by the owner 2026-11-25. Decision: `~/projects/agent-workflows/docs/our-working-model/decisions.md` ("Session host").

## Purpose and layers

```text
Lead / Conductor skills    decide: what to start, jj workspace, Beads
        │ call only
baton CLI (Rust)      run · send · keys · read · status · wait · list · close
        │ adapter
Zellij 0.46.1              zellij action … / list-panes --json        (Rex, T3 Code later)
        │
Agents in panes            Pi, Claude Code … report state themselves (OSC 7501)
```

The CLI holds mechanics only. It does not create jj workspaces, read Beads or decide what to do next. Automatic reactions (for example "start the next conductor when one is done") belong to the orchestration layer above, which calls the CLI.

## Location and stack

- Repo `~/projects/baton` (created 2026-11-25; private GitHub repo `MrAldrin/baton`). This file moved here from agent-workflows on 2026-11-25. Binary and repo name `baton` (owner choice 2026-11-25: a conductor directs with a baton; easy to dictate).
- Rust stack and check baseline: `~/projects/agent-workflows/docs/our-working-model/dev-stack.md` (section "Rust"). Dependencies so far: clap (derive), serde, serde_json, anyhow.
- `my_programs` installs it later (symlink in `~/.local/bin`). Skills call `baton`, never `zellij`, once `aw-2k0` is done.

## Verbs (v1)

Global option `--session S`; default `$ZELLIJ_SESSION_NAME`; error if neither is set. A pane is `terminal_N` or `N`. Output is one JSON object on stdout; `read` prints plain text unless `--json`. Errors: `{"error": {"message": …}}` on stdout, exit code 3.

| Verb | Does | Zellij 0.46.1 under it |
|---|---|---|
| `baton run --name N --cwd D [--tab-id T \| --new-tab NAME] [--agent pi\|claude] -- <cmd…>` | Start a program without focus. Default: a floating pane in the caller's own tab (found through `$ZELLIJ_PANE_ID`); otherwise `--tab-id` is required. `--new-tab` opens a tab whose pane runs the command directly (no shell). Records ownership. Returns `{pane, tab_id, session_id?}` | `action new-pane --floating --no-focus --tab-id --name --cwd -- cmd`; `action new-tab --no-focus --name --cwd -- cmd` |
| `baton send <pane> <text> [--no-enter] [--force]` | Type text and submit. Refuses when the state is `working` or `blocked`, unless `--force`. Records the send time | `action write-chars`, then `action write 13` |
| `baton keys <pane> <key…> [--force]` | Send keys: `enter`, `esc`, `tab`, `up`, `down`, `ctrl-c`, digits. Records the send time, like `send` | `action write <bytes>` |
| `baton read <pane> [--full] [--lines N]` | The pane's text | `action dump-screen [--full]` (prints to stdout) |
| `baton status <pane>` | `{state, source, app, message, updated_ms, exit_code}`. States: `idle`, `working`, `blocked`, `done`, `error`, `exited`, `unknown` | `list-panes --all --json` |
| `baton wait <pane> [--until done,blocked,error,exited] [--timeout S]` | Block until a listed state is reached **after the last send**. Poll every 0.5 s | `status` in a loop |
| `baton list [--owned]` | Terminal panes with name, command, cwd and state | `list-panes --all --json` |
| `baton close <pane> [--force]` | Close a pane `baton` started; `--force` for others. Removes its record | `action close-pane` |

Exit codes for `wait`: `0` done or idle, `1` error or exited, `2` blocked (waits for a person), `124` timeout, `3` other error.

## State mapping (from the spike)

- `exited: true` → `exited`, with `exit_status` as `exit_code`.
- Else the `program_status` record with the highest `updated_ms`: `Working`, `Blocked`, `Done`, `Error`, `Idle` → lower case. Source `program`.
- Else `attention.status` as a fallback (`Done`, `Failed` → `done`, `error`; others → `unknown`). Source `attention`.
- **Stale states:** a reported state stays until the agent reports again (`Done` until the next turn, `Blocked` until the agent goes on after an answer). `wait` accepts `done`, `idle`, `error` or `blocked` only when `updated_ms` ≥ the time of the last `send` or `keys`. `exited` always counts. (Changed 2026-11-25, bead `aw-i96`: `blocked` and `keys` were added, because a `wait` right after answering a prompt returned the old `Blocked` at once.)

## Ownership records

One JSON file per pane in `$XDG_STATE_HOME/baton/` (default `~/.local/state/baton/`; `%LOCALAPPDATA%` on Windows), named `<session>~terminal_N.json`: name, cwd, command, agent, session id, started_ms, last_send_ms. One file per pane avoids write races between agents. A forced `send` or `keys` to a pane `baton` did not start writes a minimal record with `owned: false`, only to keep `last_send_ms` for `wait`; the pane stays not owned. Records without `owned` count as owned.

## Agent handling (v1)

- `--agent claude` and `--agent pi` set a session id at start where the agent supports it, so a pane can be resumed later. The implementer checks the installed flags (`claude --help`, `pi --help`) and records the result.
- Checked 2026-11-25: Claude Code 2.1.316 `--session-id <uuid>` (must be a UUID; resume with `--resume <id>`); Pi 1.6.0 `--session-id <id>` (resume with `--session <id>`). `baton` adds a UUID v4 unless the command already gives a session id or resumes one.
- `run` starts every program through `env -u …` without the variables that a Claude Code host (the desktop app, or a Claude Code session) sets for its children: `CLAUDE_CODE_DISABLE_TERMINAL_TITLE`, `CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CODE_HOST_SESSION_ID`, `CLAUDE_CODE_MESSAGING_SOCKET`, `CLAUDE_CODE_MESSAGING_TOKEN`, `CLAUDE_PID` (one constant, `CLAUDE_HOST_VARS`). Inherited, they make Claude Code a child session: `CLAUDE_CODE_DISABLE_TERMINAL_TITLE` turns off its OSC 7501 reports, and its transcript is not saved. Auth and account variables are kept. Checked 2026-11-25: with them unset, Claude Code 2.1.316 reports `idle`, `working`, `done` and `blocked` (with a message such as `approve Bash: …`) as app `claude-code`.
- Windows (bead `aw-n8x.2`): a pane inherits the environment of the Zellij server, not of `baton`, so `Command::env_remove` on `baton`'s own `zellij` call cannot clear the variables. The launcher must run inside the pane. Unix keeps `env -u …`. Windows has no `env`, so `run` starts `<baton.exe> exec -- <cmd…>`: the hidden verb `exec` removes `CLAUDE_HOST_VARS` with `Command::env_remove`, starts the program, waits and exits with its exit code. `exec` ignores Ctrl-C with a console handler (not inherited), so it does not exit before its child. `--cwd` loses the `\\?\` prefix that `canonicalize` adds on Windows.
- `send` never types into a `blocked` pane without `--force`. `keys` never picks an "always" or "don't ask again" option; that choice stays with a person or the Lead.

## Rules

- Never focus a pane. Never write to or close a pane `baton` did not start, unless `--force`.
- Find the target from IDs, never from focus.

## Known limits

- Claude Code's folder-trust prompt (first start in an untrusted folder) shows as `unknown`, not `blocked`. Fix later.
- Claude Code's first-run setup (theme picker, login) appears when the Claude Code CLI was never started interactively on the machine (`~/.claude.json` has no `theme` or `hasCompletedOnboarding`), for example when only the desktop app was used. `baton` keeps `HOME`, `CLAUDE_CONFIG_DIR` and the auth variables, so this is not caused by `baton`. Run `claude` once by hand and finish the setup.
- A headless session (`attach --create-background`, no client) does not create new tabs: `new-tab` returns an id but no tab appears. Tests use floating panes in tab 0.
- `zellij events` carries no agent state, so `wait` polls. A Zellij plugin could push changes later.
- `dump-screen` of a small floating pane wraps text. Full replies are better read from the agent's own session file (later).

## Zellij 0.46.1 facts the adapter depends on

- OSC 7501 format: `ESC ] 7501 ; key=value:key=value ESC \`. Keys are separated by `:`, not `;` (with `;` Zellij ignores the whole sequence). Keys: `state`, `app`, `kind`, `msg`; `msg` is base64. Seen in Pi 1.6.0's source and checked against Zellij.
- `zellij --session S action …` exits 0 for an unknown session (it prints the session list on stdout) and for an unknown pane id. `baton` checks that the session (`list-sessions`) and the pane (`list-panes`) exist first.
- An exited pane clears `program_status`; `attention.status` is then `Failed`, so the `exited` check comes first.

- `--new-tab` gives the command to `new-tab -- cmd…`, so the user's default shell (Nushell, PowerShell, cmd.exe, fish …) never parses it. Typing a quoted line into the shell failed in Nushell (`'C:\…aton.exe' exec …` needs `^'…'` there). When the program ends, the pane shows it as exited, as a floating pane does.
- Windows: `baton exec` finds the program through `PATH` and `PATHEXT`, so npm shims such as `pi.cmd` start (Rust runs a `.cmd`/`.bat` path through cmd.exe). An extensionless POSIX shim next to it is never picked.
- Windows (ConPTY) needs Zellij 0.46.1. Checked live with Pi 1.6.1 and Claude Code 2.1.332: OSC 7501 output passes through ConPTY to Zellij, and `list-panes --json` and `dump-screen` work. With Zellij 0.46.0 the answer to the agents' feature query (`ESC ] 7501 ; ? ESC \`) is lost, so they report no state (Claude Code has no override and stays `unknown`). Zellij 0.46.1 delivers the answer: Claude Code and Pi reported `idle` → `done`, floating and `--new-tab`. `run --agent pi` still starts Pi with `PI_PROGRAM_STATUS=1` (`exec --default-env`) unless the pane sets it; this is harmless and helps on 0.46.0.

## Not in v1

`events` stream, SSH (`ssh host baton …`), status hooks, Claude Code background sessions, Rex and T3 Code adapters, a Zellij plugin.

## Tests (acceptance)

- Check baseline passes: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` (pedantic as warnings), `cargo nextest run`, `cargo test --doc`.
- Unit tests: state mapping and the stale-`Done` rule, from recorded `list-panes` JSON fixtures.
- Integration tests (assert_cmd) against a throwaway background session `baton-test-<pid>`, deleted afterwards. A fake agent (`examples/fake_agent.rs`, Rust so it runs on Windows too) prints OSC 7501 (`ESC ] 7501 ; state=working:app=fake ESC \` … `state=done`) so tests need no model. Tests that need Zellij 0.46.1 skip when it is not on `PATH`, unless `BATON_REQUIRE_ZELLIJ` is set.
- CI (`.github/workflows/ci.yml`): `ubuntu-latest` and `windows-latest` run the check baseline. Linux installs Zellij 0.46.1 and requires the integration tests; Windows runs them against the official Windows build as best effort. Cover run, send, keys, read, status, wait (including stale `Done` and timeout), list, close and the ownership refusal.
- One manual live check each with Pi and Claude Code: run, send, wait, read.

## Background

- [Spike: agent state in Zellij 0.46.1](background/zellij-agent-state-spike.md)
- Prior art: [zaag](https://github.com/IsmailHatim/zaag) (same verbs, Python; the reference), [rex-orchestrator](https://github.com/dcsg/rex-orchestrator) (safety rules), [zjctl](https://github.com/mrshu/zjctl) (Rust, screen-based wait). Compared on 2026-11-25: zaag reads `program_status` on Zellij 0.46 and waits for the new turn; its installer edits `~/.claude/settings.json` and `config.kdl`, so it was not installed.
- A plugin is WASM inside Zellij, for UI or push events; the verbs need none.
