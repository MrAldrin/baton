---
name: using-baton
description: >
  Use when an agent starts, prompts, waits for, reads or closes another agent (Pi, Claude Code) in a Zellij pane with the baton CLI.
---

# Using baton

`baton` starts and drives agents in Zellij 0.46.1 panes. Use it, not `zellij action`, to control another agent. The spec is the authority: [`docs/design.md`](../../docs/design.md).

## Before you start

- On Windows, Zellij must be 0.46.1 or later; with 0.46.0 Claude Code stays `unknown` and `wait` cannot work.
- baton needs a session: `--session S`, or `$ZELLIJ_SESSION_NAME` (set inside Zellij). With neither, it stops with an error.
- A pane is `terminal_N` or `N`. Use the pane ID that `run` returns. Never find a pane from focus.
- Output is one JSON object on stdout. `read` prints plain text unless `--json`. Errors print `{"error": {"message": …}}` and exit `3`.

## Verbs

| Verb | Example |
|---|---|
| `run` | `baton run --name worker --cwd ~/proj --agent claude` |
| `send` | `baton send terminal_3 "Fix the failing test"` |
| `keys` | `baton keys terminal_3 down enter` |
| `wait` | `baton wait terminal_3 --timeout 600` |
| `read` | `baton read terminal_3 --lines 40` |
| `status` | `baton status terminal_3` |
| `list` | `baton list --owned` (find a pane by `.name`, the `run --name`; `.title` is the live title) |
| `close` | `baton close terminal_3` |

- `run` opens a floating pane without focus. Default: in the caller's own tab. Outside Zellij, give `--tab-id T`. `--new-tab NAME` opens a new tab that runs the command directly (any default shell works).
- If Claude Code stops at its first-run theme picker, the CLI was never set up on that machine. Ask the user to run `claude` once by hand; do not pick options for them.
- `run` prints `{pane, tab_id, session_id?}`. Record the pane and the session ID.
- `run --agent pi|claude` with no command starts that agent. With a command after `--`, it adds a session ID to that command.
- `send --no-enter` types the text and does not submit it.
- `keys` accepts `enter`, `esc`, `tab`, `up`, `down`, `ctrl-c` and digits.
- `read --full` includes the scrollback.
- `status` prints `{state, source, app, message, updated_ms, exit_code}`. States: `idle`, `working`, `blocked`, `done`, `error`, `exited`, `unknown`.
- `wait --until` sets the states to stop on. Default: `done,blocked,error,exited`.

## Agents and session IDs

- Use `--agent pi` or `--agent claude` for every agent. baton then sets a session ID, so the pane can be resumed later: `pi --session <id>` or `claude --resume <id>`.
- Pi and Claude Code report their state themselves. baton removes the Claude Code host variables, so a started Claude Code reports its state.
- On Windows, `--agent pi` also sets `PI_PROGRAM_STATUS=1`, so Pi reports its state through ConPTY. Started without `--agent pi`, Pi stays `unknown` and `wait` times out.
- In a new, untrusted folder, Claude Code first asks for trust. That prompt shows as `unknown`, not `blocked`. Read the pane to see it.

## Send, then wait

A state stays until the agent reports again. An old `done` or `blocked` is still there after you send.

- Always `send` (or `keys`), then `wait`. `wait` ignores a `done`, `idle`, `error` or `blocked` state from before the last `send` or `keys`.
- Do not use `status` to find out if a new turn is finished. Use `wait`.
- After `wait`, use `read` to get the reply.

Exit codes of `wait`:

| Code | Meaning | Do |
|---|---|---|
| `0` | `done` or `idle` | Read the reply. |
| `1` | `error` or `exited` | Read the pane; report. |
| `2` | `blocked`: the agent waits for a person | Read the prompt. Ask before you answer it. |
| `124` | Timeout | Check `status` and `read`; wait again or report. |
| `3` | Other error | Read the JSON error. |

## Safety rules

- Never send to a `blocked` pane without asking the person first. `send` refuses `working` and `blocked` panes; do not add `--force` to get past that.
- `keys` never picks an "always" or "don't ask again" option. That choice stays with a person.
- Never write to or close a pane that baton did not start. `--force` is only for a pane the person tells you to use.
- Never focus a pane.
- Close the panes you started when their work is done.

## What baton does not do

baton holds mechanics only. It does not:

- create or clean up jj workspaces,
- read or update Beads,
- decide what to start, send or do next. The calling agent or the person decides.
