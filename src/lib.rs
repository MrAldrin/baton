//! `baton`: a thin CLI over a terminal multiplexer, for agents in panes.
//!
//! The verbs (`run`, `send`, `keys`, `read`, `status`, `wait`, `list`,
//! `close`) live in [`commands`]. They talk to the multiplexer only through
//! the [`host::Host`] trait. [`zellij::Zellij`] is the only adapter in v1.

pub mod agent;
pub mod commands;
pub mod host;
pub mod keys;
pub mod launch;
pub mod pane;
pub mod state;
pub mod store;
pub mod zellij;

/// Milliseconds since the Unix epoch, the same clock Zellij uses for
/// `updated_ms`.
#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
