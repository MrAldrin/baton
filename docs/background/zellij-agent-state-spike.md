# Spike: agent state in Zellij 0.46.1

Bead `aw-u5a`, 2026-11-25. Zellij 0.46.1, Pi 1.6.0, Linux. One Pi agent ran in a background session (`aw-spike`), and the session was then deleted. The owner's sessions were not touched.

## Result

The run → send → wait → read loop works on Zellij alone. No Pi extension and no screen guessing are needed.

| Need | Zellij 0.46.1 command | Works? |
|---|---|---|
| Start an agent without focus | `zellij --session S action new-pane --floating --name N --cwd D -- pi "<prompt>"` → prints `terminal_1` | Yes |
| Agent state | `zellij action list-panes --all --json` → `program_status[0].state` and `attention.status` per pane | Yes. Pi reports `Working` and `Done` through OSC 7501 |
| Send input | `action write-chars --pane-id terminal_1 "<text>"`, then `action write --pane-id terminal_1 13` (Enter) | Yes |
| Read the reply | `action dump-screen --pane-id terminal_1` | Yes, but a small floating pane wraps text. Pi's session file is the better source for full replies |
| Structure events | `zellij events` (NDJSON) | Yes: `initial_state`, `pane_opened`, `pane_command_changed` and others |
| State events | `zellij events` | **No.** Agent state (OSC 7501, attention) is not an event. Read it with `list-panes` |

## What the state looked like

Second turn, polled every 0.5 s after sending input:

```text
 4 × Working  (source ProgramStatus)
36 × Done     (source ProgramStatus)
```

Before Pi's first report, `attention.status` was `NewOutput` (source `UnseenOutput`). After that, `ProgramStatus` was the source. `program_status[0]` also has `app: "pi"` and `updated_ms`.

## Things the host CLI must handle

- **`Done` stays until the next turn.** A `wait` right after `send` can see the old `Done` and return at once. Fix: record the time of `send`, then accept `Done` only when `updated_ms` is newer, or after a `Working` was seen.
- **State needs polling.** `zellij events` does not carry state. `wait` polls `list-panes`; 0.5 s was fast enough to see `Working` on a short turn. A Zellij plugin could push state changes later, if polling costs too much.
- **Enter is a separate write.** `write-chars` does not submit. Send byte 13 with `action write`.
- **Other built-in tools seen but not tested:** `new-pane --notify` and `action watch-pane` (notify when a command finishes or output goes quiet), `--blocking` (block until the command exits), `mute-pane`, and `zellij subscribe` (stream the screen text of a pane).
