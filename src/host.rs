//! The adapter boundary. Each multiplexer (Zellij now; Rex or T3 Code later)
//! implements [`Host`].

use std::path::Path;

use anyhow::{Result, anyhow};
use serde::Serialize;

use crate::pane::PaneId;
use crate::state::Status;

/// One terminal pane, as the verbs see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaneInfo {
    pub pane: PaneId,
    pub name: String,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub tab_id: Option<u64>,
    pub floating: bool,
    pub status: Status,
}

/// A request to start a program in a new pane without focus.
#[derive(Debug, Clone)]
pub struct Spawn<'a> {
    pub name: &'a str,
    pub cwd: &'a Path,
    pub tab_id: u64,
    pub command: &'a [String],
}

/// What `baton` needs from a multiplexer. No call may change focus.
pub trait Host {
    /// The session this host talks to.
    fn session(&self) -> &str;

    /// All terminal panes (no plugins) in the session.
    ///
    /// # Errors
    /// The multiplexer fails or returns output that cannot be parsed.
    fn list_panes(&self) -> Result<Vec<PaneInfo>>;

    /// Start a program in a new floating pane in `spawn.tab_id`.
    ///
    /// # Errors
    /// The multiplexer refuses or does not return a pane id.
    fn spawn_floating(&self, spawn: &Spawn<'_>) -> Result<PaneId>;

    /// Open a new tab in `cwd` whose pane runs `command` directly (no
    /// shell in between), and return its tab id.
    ///
    /// # Errors
    /// The multiplexer refuses or does not return a tab id.
    fn new_tab(&self, name: &str, cwd: &Path, command: &[String]) -> Result<u64>;

    /// Type text into a pane, without Enter.
    ///
    /// # Errors
    /// The multiplexer fails.
    fn write_text(&self, pane: PaneId, text: &str) -> Result<()>;

    /// Write raw bytes into a pane.
    ///
    /// # Errors
    /// The multiplexer fails.
    fn write_bytes(&self, pane: PaneId, bytes: &[u8]) -> Result<()>;

    /// The pane's screen text; with `full`, also the scrollback.
    ///
    /// # Errors
    /// The multiplexer fails.
    fn dump_screen(&self, pane: PaneId, full: bool) -> Result<String>;

    /// Close a pane.
    ///
    /// # Errors
    /// The multiplexer fails.
    fn close_pane(&self, pane: PaneId) -> Result<()>;

    /// Find one pane. Multiplexers may accept actions on unknown panes
    /// without an error, so verbs call this first.
    ///
    /// # Errors
    /// The pane does not exist, or [`Host::list_panes`] fails.
    fn find_pane(&self, pane: PaneId) -> Result<PaneInfo> {
        self.list_panes()?
            .into_iter()
            .find(|p| p.pane == pane)
            .ok_or_else(|| anyhow!("pane {pane} not found in session {}", self.session()))
    }
}
