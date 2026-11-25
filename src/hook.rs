//! State reports for agents that do not emit OSC 7501 themselves.
//!
//! Claude Code 2.1.316 emits no OSC 7501. `baton run --agent claude` adds
//! hooks (through `--settings`) that call `baton hook claude`. That command
//! reads the hook input on stdin, maps the event to a state and writes the
//! OSC 7501 sequence to Claude's own terminal.
//!
//! Hooks run without a controlling terminal (`/dev/tty` fails), so the
//! terminal is found through `/proc`: the first ancestor whose stdout is a
//! pty. This works on Linux only; elsewhere the hook does nothing.

use std::fmt::Write as _;

use serde_json::Value;

use crate::state::State;

/// One state report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub state: State,
    /// Zellij `kind` for `blocked` (for example `permission`).
    pub kind: Option<&'static str>,
    pub message: Option<String>,
}

impl Report {
    fn new(state: State) -> Self {
        Self {
            state,
            kind: None,
            message: None,
        }
    }
}

/// The Claude Code hook events `baton` listens to.
pub const CLAUDE_EVENTS: [&str; 9] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "Notification",
    "Stop",
    "StopFailure",
];

/// Map a Claude Code hook input to a state. `None`: no change.
#[must_use]
pub fn claude_report(input: &Value) -> Option<Report> {
    let text = |key: &str| input.get(key).and_then(Value::as_str);
    let tool = text("tool_name");
    match text("hook_event_name")? {
        "SessionStart" => Some(Report::new(State::Idle)),
        "UserPromptSubmit" | "PostToolUse" | "PostToolUseFailure" => {
            Some(Report::new(State::Working))
        }
        // AskUserQuestion waits for a person.
        "PreToolUse" if tool == Some("AskUserQuestion") => Some(Report {
            message: Some("question".into()),
            ..Report::new(State::Blocked)
        }),
        "PreToolUse" => Some(Report::new(State::Working)),
        "PermissionRequest" => Some(Report {
            state: State::Blocked,
            kind: Some("permission"),
            message: tool.map(str::to_owned),
        }),
        "Notification" => {
            let kind = text("notification_type").unwrap_or_default();
            let message = text("message").unwrap_or_default();
            let asks = matches!(kind, "permission_prompt" | "elicitation_dialog")
                || message.contains("needs your permission");
            asks.then(|| Report {
                state: State::Blocked,
                kind: (kind != "elicitation_dialog").then_some("permission"),
                message: Some(message.to_owned()).filter(|m| !m.is_empty()),
            })
        }
        "Stop" => Some(Report::new(State::Done)),
        "StopFailure" => Some(Report {
            message: text("error").map(str::to_owned),
            ..Report::new(State::Error)
        }),
        _ => None,
    }
}

/// The OSC 7501 sequence for a report. Keys are separated by `:`, and the
/// message is base64 (the format Pi 1.6.0 uses and Zellij 0.46.1 parses).
///
/// ```
/// use baton::hook::{Report, osc};
/// use baton::state::State;
/// let r = Report { state: State::Done, kind: None, message: Some("ok".into()) };
/// assert_eq!(osc(&r, "claude"), "\x1b]7501;state=done:app=claude:msg=b2s=\x1b\\");
/// ```
#[must_use]
pub fn osc(report: &Report, app: &str) -> String {
    let state = serde_json::to_value(report.state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let mut s = format!("\x1b]7501;state={state}:app={app}");
    if let Some(kind) = report.kind {
        let _ = write!(s, ":kind={kind}");
    }
    if let Some(msg) = report.message.as_deref().filter(|m| !m.is_empty()) {
        let msg: String = msg.chars().filter(|c| !c.is_control()).take(500).collect();
        let _ = write!(s, ":msg={}", base64(msg.as_bytes()));
    }
    s.push_str("\x1b\\");
    s
}

/// Standard base64 with padding.
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = c
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (u32::from(b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= c.len() {
                out.push(char::from(T[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Write bytes to the agent's terminal: the first ancestor process (at
/// most 6 levels up) whose stdout is a pty. Returns false if none is found.
#[must_use]
pub fn write_to_agent_terminal(bytes: &[u8]) -> bool {
    #[cfg(target_os = "linux")]
    {
        use std::io::Write;
        let mut pid = std::os::unix::process::parent_id();
        for _ in 0..6 {
            if pid <= 1 {
                break;
            }
            let fd = std::fs::read_link(format!("/proc/{pid}/fd/1")).ok();
            if let Some(path) = fd.filter(|p| p.starts_with("/dev/pts/")) {
                return std::fs::OpenOptions::new()
                    .write(true)
                    .open(path)
                    .and_then(|mut f| f.write_all(bytes))
                    .is_ok();
            }
            match parent_of(pid) {
                Some(p) => pid = p,
                None => break,
            }
        }
        false
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = bytes;
        false
    }
}

#[cfg(target_os = "linux")]
fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Field 4, after the command name in parentheses.
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// The `--settings` JSON that makes Claude Code call `<baton> hook claude`
/// on each event in [`CLAUDE_EVENTS`].
#[must_use]
pub fn claude_settings(hook_command: &str) -> String {
    let entry = serde_json::json!([{
        "hooks": [{ "type": "command", "command": hook_command, "timeout": 5 }]
    }]);
    let hooks: serde_json::Map<String, Value> = CLAUDE_EVENTS
        .iter()
        .map(|e| ((*e).to_owned(), entry.clone()))
        .collect();
    serde_json::json!({ "hooks": hooks }).to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn state(input: &Value) -> Option<State> {
        claude_report(input).map(|r| r.state)
    }

    #[test]
    fn claude_events() {
        let ev = |name: &str| json!({ "hook_event_name": name });
        assert_eq!(state(&ev("SessionStart")), Some(State::Idle));
        assert_eq!(state(&ev("UserPromptSubmit")), Some(State::Working));
        assert_eq!(state(&ev("PostToolUse")), Some(State::Working));
        assert_eq!(state(&ev("Stop")), Some(State::Done));
        assert_eq!(state(&ev("StopFailure")), Some(State::Error));
        assert_eq!(state(&ev("SessionEnd")), None);
        assert_eq!(state(&json!({})), None);
    }

    #[test]
    fn claude_blocked() {
        let r = claude_report(&json!({
            "hook_event_name": "PermissionRequest", "tool_name": "Bash"
        }))
        .unwrap();
        assert_eq!(r.state, State::Blocked);
        assert_eq!(r.kind, Some("permission"));
        assert_eq!(r.message.as_deref(), Some("Bash"));

        let ask = json!({ "hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion" });
        assert_eq!(state(&ask), Some(State::Blocked));
        let read = json!({ "hook_event_name": "PreToolUse", "tool_name": "Read" });
        assert_eq!(state(&read), Some(State::Working));

        let note = |t: &str| json!({ "hook_event_name": "Notification", "notification_type": t });
        assert_eq!(state(&note("permission_prompt")), Some(State::Blocked));
        assert_eq!(state(&note("elicitation_dialog")), Some(State::Blocked));
        assert_eq!(state(&note("idle_prompt")), None);
    }

    #[test]
    fn base64_matches_known_values() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"need ok"), "bmVlZCBvaw==");
    }

    #[test]
    fn osc_blocked() {
        let r = Report {
            state: State::Blocked,
            kind: Some("permission"),
            message: Some("need ok".into()),
        };
        assert_eq!(
            osc(&r, "claude"),
            "\x1b]7501;state=blocked:app=claude:kind=permission:msg=bmVlZCBvaw==\x1b\\"
        );
    }

    #[test]
    fn settings_cover_all_events() {
        let v: Value = serde_json::from_str(&claude_settings("/x/baton hook claude")).unwrap();
        for e in CLAUDE_EVENTS {
            assert_eq!(
                v["hooks"][e][0]["hooks"][0]["command"],
                "/x/baton hook claude"
            );
        }
    }
}
