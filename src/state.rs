//! Pane state, independent of the multiplexer, and the rules `wait` uses.

use serde::{Deserialize, Serialize};

/// The state of a pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Idle,
    Working,
    Blocked,
    Done,
    Error,
    Exited,
    Unknown,
}

/// Where a state came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// The program's own report (OSC 7501).
    Program,
    /// The multiplexer's attention record (fallback).
    Attention,
    /// The pane itself: its program exited.
    Pane,
    /// No information.
    None,
}

/// The output of `baton status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub state: State,
    pub source: Source,
    pub app: Option<String>,
    pub message: Option<String>,
    pub updated_ms: Option<u64>,
    pub exit_code: Option<i32>,
}

impl Status {
    /// A status with no information.
    #[must_use]
    pub fn unknown() -> Self {
        Self {
            state: State::Unknown,
            source: Source::None,
            app: None,
            message: None,
            updated_ms: None,
            exit_code: None,
        }
    }

    /// True when `wait` may stop on this status.
    ///
    /// The state must be in `until`. `done`, `idle` and `error` count only
    /// when `updated_ms` is at or after the last send, because `Done` stays
    /// latched until the next turn (stale `Done`). Other states always count.
    #[must_use]
    pub fn satisfies(&self, until: &[State], last_send_ms: Option<u64>) -> bool {
        if !until.contains(&self.state) {
            return false;
        }
        match (self.state, last_send_ms) {
            (State::Done | State::Idle | State::Error, Some(sent)) => {
                self.updated_ms.is_some_and(|u| u >= sent)
            }
            _ => true,
        }
    }
}

/// States that `wait --until` accepts.
pub const WAITABLE: [State; 5] = [
    State::Done,
    State::Idle,
    State::Blocked,
    State::Error,
    State::Exited,
];

/// Exit code of `wait` for a reached state.
#[must_use]
pub fn wait_exit_code(state: State) -> i32 {
    match state {
        State::Done | State::Idle => 0,
        State::Error | State::Exited => 1,
        State::Blocked => 2,
        State::Working | State::Unknown => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(state: State, updated_ms: Option<u64>) -> Status {
        Status {
            state,
            updated_ms,
            ..Status::unknown()
        }
    }

    const ALL: [State; 4] = [State::Done, State::Blocked, State::Error, State::Exited];

    #[test]
    fn done_before_send_is_stale() {
        assert!(!st(State::Done, Some(100)).satisfies(&ALL, Some(200)));
    }

    #[test]
    fn done_at_or_after_send_counts() {
        assert!(st(State::Done, Some(200)).satisfies(&ALL, Some(200)));
        assert!(st(State::Done, Some(201)).satisfies(&ALL, Some(200)));
    }

    #[test]
    fn done_without_send_counts() {
        assert!(st(State::Done, Some(1)).satisfies(&ALL, None));
    }

    #[test]
    fn error_and_idle_follow_the_stale_rule() {
        let until = [State::Idle, State::Error];
        assert!(!st(State::Error, Some(1)).satisfies(&until, Some(2)));
        assert!(!st(State::Idle, None).satisfies(&until, Some(2)));
        assert!(st(State::Idle, Some(3)).satisfies(&until, Some(2)));
    }

    #[test]
    fn exited_always_counts() {
        assert!(st(State::Exited, None).satisfies(&ALL, Some(500)));
    }

    #[test]
    fn state_not_in_until_never_counts() {
        assert!(!st(State::Working, Some(900)).satisfies(&ALL, Some(1)));
        assert!(!st(State::Idle, Some(900)).satisfies(&ALL, Some(1)));
    }

    #[test]
    fn exit_codes() {
        assert_eq!(wait_exit_code(State::Done), 0);
        assert_eq!(wait_exit_code(State::Idle), 0);
        assert_eq!(wait_exit_code(State::Error), 1);
        assert_eq!(wait_exit_code(State::Exited), 1);
        assert_eq!(wait_exit_code(State::Blocked), 2);
    }
}
