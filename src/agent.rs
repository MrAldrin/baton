//! Agent-specific start options.
//!
//! Checked 2026-11-25: Claude Code 2.1.316 has `--session-id <uuid>` (must
//! be a UUID); Pi 1.6.0 has `--session-id <id>` ("use the current
//! directory's existing session with the ID, creating it otherwise").
//! Both resume later with `--resume <id>` (Claude) or `--session <id>` (Pi).

use serde::{Deserialize, Serialize};

/// An agent that `run --agent` knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Pi,
    Claude,
}

impl Agent {
    /// The program name, used when `run` gets no command.
    #[must_use]
    pub fn program(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Claude => "claude",
        }
    }
}

/// Flags that already choose a session; `baton` then adds no id.
const SESSION_FLAGS: [&str; 6] = ["--resume", "-r", "--continue", "-c", "--session", "--fork"];

/// Add a session id to an agent command where the agent supports it.
///
/// Returns the command to run and the session id, if known. An id the
/// caller already gave with `--session-id` is kept. When the command
/// already resumes or continues a session, nothing is added.
///
/// ```
/// use baton::agent::{Agent, with_session_id};
/// let (cmd, id) = with_session_id(Agent::Pi, vec!["pi".into(), "hi".into()], || "x1".into());
/// assert_eq!(cmd, ["pi", "--session-id", "x1", "hi"]);
/// assert_eq!(id.as_deref(), Some("x1"));
/// ```
pub fn with_session_id(
    agent: Agent,
    mut command: Vec<String>,
    new_id: impl FnOnce() -> String,
) -> (Vec<String>, Option<String>) {
    if command.is_empty() {
        command.push(agent.program().to_owned());
    }
    if let Some(i) = command.iter().position(|a| a == "--session-id") {
        let id = command.get(i + 1).cloned();
        return (command, id);
    }
    if let Some(id) = command.iter().find_map(|a| a.strip_prefix("--session-id=")) {
        let id = id.to_owned();
        return (command, Some(id));
    }
    if command
        .iter()
        .skip(1)
        .any(|a| SESSION_FLAGS.contains(&a.as_str()) || a.starts_with("--resume="))
    {
        return (command, None);
    }
    let id = new_id();
    command.splice(1..1, ["--session-id".to_owned(), id.clone()]);
    (command, Some(id))
}

/// A random UUID v4 (Claude Code needs a UUID).
#[must_use]
pub fn new_uuid() -> String {
    let mut b = random_16();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = b.iter().fold(String::with_capacity(32), |mut h, x| {
        use std::fmt::Write;
        let _ = write!(h, "{x:02x}");
        h
    });
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

fn random_16() -> [u8; 16] {
    use std::hash::{BuildHasher, Hasher};
    use std::io::Read;
    let mut b = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_ok()
    {
        return b;
    }
    // No /dev/urandom (Windows): mix time, pid and std's random hasher keys.
    for (i, chunk) in b.chunks_mut(8).enumerate() {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::UNIX_EPOCH.elapsed().map_or(0, |d| d.as_nanos()));
        h.write_u32(std::process::id());
        h.write_usize(i);
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn empty_command_uses_the_agent() {
        let (cmd, id) = with_session_id(Agent::Claude, vec![], || "u".into());
        assert_eq!(cmd, v(&["claude", "--session-id", "u"]));
        assert_eq!(id.as_deref(), Some("u"));
    }

    #[test]
    fn given_id_is_kept() {
        let (cmd, id) = with_session_id(
            Agent::Pi,
            v(&["pi", "--session-id", "abc"]),
            || unreachable!(),
        );
        assert_eq!(cmd, v(&["pi", "--session-id", "abc"]));
        assert_eq!(id.as_deref(), Some("abc"));
        let (_, id) = with_session_id(Agent::Pi, v(&["pi", "--session-id=xyz"]), || unreachable!());
        assert_eq!(id.as_deref(), Some("xyz"));
    }

    #[test]
    fn resume_adds_nothing() {
        let (cmd, id) = with_session_id(
            Agent::Claude,
            v(&["claude", "--resume", "r1"]),
            || unreachable!(),
        );
        assert_eq!(cmd, v(&["claude", "--resume", "r1"]));
        assert_eq!(id, None);
    }

    #[test]
    fn uuid_shape() {
        let u = new_uuid();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(u, new_uuid());
    }
}
