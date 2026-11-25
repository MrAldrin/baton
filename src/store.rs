//! Ownership records: one JSON file per pane that `baton` started.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::agent::Agent;
use crate::pane::PaneId;

/// What `baton run` recorded about a pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub session: String,
    pub pane: PaneId,
    pub name: String,
    pub cwd: PathBuf,
    pub command: Vec<String>,
    pub agent: Option<Agent>,
    pub session_id: Option<String>,
    pub tab_id: u64,
    pub started_ms: u64,
    pub last_send_ms: Option<u64>,
}

/// The folder with the records.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// `$XDG_STATE_HOME/baton`, else `~/.local/state/baton`
    /// (`%LOCALAPPDATA%\baton` on Windows).
    ///
    /// # Errors
    /// No home folder is known.
    pub fn default_location() -> Result<Self> {
        let base = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_STATE_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        };
        let base = base.ok_or_else(|| anyhow!("cannot find a state folder: set HOME"))?;
        Ok(Self::at(base.join("baton")))
    }

    /// A store in `dir`.
    #[must_use]
    pub fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, session: &str, pane: PaneId) -> PathBuf {
        self.dir.join(format!("{session}~{pane}.json"))
    }

    /// The record for a pane, if `baton` started it.
    ///
    /// # Errors
    /// The file exists but cannot be read or parsed.
    pub fn load(&self, session: &str, pane: PaneId) -> Result<Option<Record>> {
        let path = self.path(session, pane);
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .with_context(|| format!("cannot parse {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    /// Write a record (through a temporary file and a rename).
    ///
    /// # Errors
    /// The folder or file cannot be written.
    pub fn save(&self, rec: &Record) -> Result<()> {
        fs::create_dir_all(&self.dir)
            .with_context(|| format!("cannot create {}", self.dir.display()))?;
        let path = self.path(&rec.session, rec.pane);
        let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        let text = serde_json::to_string_pretty(rec)?;
        fs::write(&tmp, text).with_context(|| format!("cannot write {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("cannot write {}", path.display()))
    }

    /// Remove a record. A missing record is not an error.
    ///
    /// # Errors
    /// The file exists but cannot be removed.
    pub fn remove(&self, session: &str, pane: PaneId) -> Result<()> {
        let path = self.path(session, pane);
        match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(e).with_context(|| format!("cannot remove {}", path.display()))
            }
            _ => Ok(()),
        }
    }

    /// The folder.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::at(tmp.path().join("baton"));
        let rec = Record {
            session: "s".into(),
            pane: PaneId(4),
            name: "n".into(),
            cwd: "/w".into(),
            command: vec!["pi".into()],
            agent: Some(Agent::Pi),
            session_id: Some("id".into()),
            tab_id: 0,
            started_ms: 1,
            last_send_ms: None,
        };
        assert_eq!(store.load("s", PaneId(4)).unwrap(), None);
        store.save(&rec).unwrap();
        assert!(tmp.path().join("baton/s~terminal_4.json").exists());
        assert_eq!(store.load("s", PaneId(4)).unwrap(), Some(rec));
        store.remove("s", PaneId(4)).unwrap();
        store.remove("s", PaneId(4)).unwrap();
        assert_eq!(store.load("s", PaneId(4)).unwrap(), None);
    }
}
