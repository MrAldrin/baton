//! Zellij 0.46.1 adapter.

use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::host::{Host, PaneInfo, Spawn};
use crate::pane::PaneId;
use crate::state::{Source, State, Status};

/// A Zellij session, driven through `zellij --session S action …`.
#[derive(Debug, Clone)]
pub struct Zellij {
    session: String,
}

impl Zellij {
    /// Connect to a running session.
    ///
    /// Zellij prints a session list and exits 0 when the session does not
    /// exist, so this checks `list-sessions` first.
    ///
    /// # Errors
    /// The session does not exist or has exited, or `zellij` cannot run.
    pub fn connect(session: &str) -> Result<Self> {
        let out = Command::new("zellij")
            .args(["list-sessions", "--no-formatting"])
            .output()
            .context("cannot run zellij")?;
        let text = String::from_utf8_lossy(&out.stdout);
        if !out.status.success() {
            // With no sessions at all, Zellij exits 1.
            let err = String::from_utf8_lossy(&out.stderr);
            if format!("{text}{err}").contains("No active zellij sessions") {
                bail!("Zellij session {session:?} not found");
            }
            bail!(
                "cannot list Zellij sessions ({}): {}",
                out.status,
                err.trim()
            );
        }
        let line = text
            .lines()
            .find(|l| l.split_whitespace().next() == Some(session));
        match line {
            None => bail!("Zellij session {session:?} not found"),
            Some(l) if l.contains("EXITED") => bail!("Zellij session {session:?} has exited"),
            Some(_) => Ok(Self {
                session: session.to_owned(),
            }),
        }
    }

    fn action(&self, args: &[&str]) -> Result<Output> {
        let mut cmd = Command::new("zellij");
        cmd.args(["--session", &self.session, "action"]).args(args);
        run(&mut cmd).with_context(|| format!("zellij action {}", args.first().unwrap_or(&"")))
    }
}

fn run(cmd: &mut Command) -> Result<Output> {
    let out = cmd.output().context("cannot run zellij")?;
    if !out.status.success() {
        bail!(
            "zellij failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out)
}

impl Host for Zellij {
    fn session(&self) -> &str {
        &self.session
    }

    fn list_panes(&self) -> Result<Vec<PaneInfo>> {
        let out = self.action(&["list-panes", "--all", "--json"])?;
        parse_panes(&String::from_utf8_lossy(&out.stdout))
    }

    fn spawn_floating(&self, spawn: &Spawn<'_>) -> Result<PaneId> {
        let tab = spawn.tab_id.to_string();
        let cwd = spawn.cwd.to_string_lossy();
        let mut args = vec![
            "new-pane",
            "--floating",
            "--no-focus",
            "--tab-id",
            &tab,
            "--name",
            spawn.name,
            "--cwd",
            &cwd,
            "--",
        ];
        args.extend(spawn.command.iter().map(String::as_str));
        let out = self.action(&args)?;
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .context("zellij new-pane did not return a terminal pane id")
    }

    fn new_tab(&self, name: &str, cwd: &Path) -> Result<u64> {
        let cwd = cwd.to_string_lossy();
        let out = self.action(&["new-tab", "--no-focus", "--name", name, "--cwd", &cwd])?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.trim()
            .parse()
            .with_context(|| format!("zellij new-tab did not return a tab id: {text:?}"))
    }

    fn write_text(&self, pane: PaneId, text: &str) -> Result<()> {
        let id = pane.to_string();
        self.action(&["write-chars", "--pane-id", &id, "--", text])?;
        Ok(())
    }

    fn write_bytes(&self, pane: PaneId, bytes: &[u8]) -> Result<()> {
        let id = pane.to_string();
        let nums: Vec<String> = bytes.iter().map(u8::to_string).collect();
        let mut args = vec!["write", "--pane-id", &id, "--"];
        args.extend(nums.iter().map(String::as_str));
        self.action(&args)?;
        Ok(())
    }

    fn dump_screen(&self, pane: PaneId, full: bool) -> Result<String> {
        let id = pane.to_string();
        let mut args = vec!["dump-screen", "--pane-id", &id];
        if full {
            args.push("--full");
        }
        let out = self.action(&args)?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn close_pane(&self, pane: PaneId) -> Result<()> {
        let id = pane.to_string();
        self.action(&["close-pane", "--pane-id", &id])?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct RawPane {
    id: u32,
    is_plugin: bool,
    #[serde(default)]
    is_floating: bool,
    #[serde(default)]
    title: String,
    #[serde(default)]
    exited: bool,
    exit_status: Option<i32>,
    attention: Option<RawAttention>,
    #[serde(default)]
    program_status: Vec<RawProgram>,
    tab_id: Option<u64>,
    terminal_command: Option<String>,
    pane_command: Option<String>,
    pane_cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawAttention {
    status: Option<String>,
    changed_ms: Option<u64>,
    detail: Option<RawDetail>,
}

#[derive(Debug, Deserialize)]
struct RawDetail {
    app: Option<String>,
    body: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawProgram {
    state: Option<String>,
    app: Option<String>,
    msg: Option<String>,
    updated_ms: Option<u64>,
}

/// Parse `zellij action list-panes --all --json` into terminal panes.
///
/// # Errors
/// The text is not the expected JSON.
pub fn parse_panes(json: &str) -> Result<Vec<PaneInfo>> {
    let raw: Vec<RawPane> =
        serde_json::from_str(json).context("cannot parse zellij list-panes JSON")?;
    Ok(raw
        .into_iter()
        .filter(|p| !p.is_plugin)
        .map(|p| PaneInfo {
            pane: PaneId(p.id),
            status: map_status(&p),
            name: p.title,
            command: p.pane_command.or(p.terminal_command),
            cwd: p.pane_cwd,
            tab_id: p.tab_id,
            floating: p.is_floating,
        })
        .collect())
}

/// The state mapping from the spec:
/// 1. `exited` → `exited`, with `exit_status` as `exit_code`.
/// 2. The `program_status` record with the highest `updated_ms`.
/// 3. `attention.status` as a fallback: `Done` → `done`, `Failed` →
///    `error`, others → `unknown`.
fn map_status(p: &RawPane) -> Status {
    if p.exited {
        return Status {
            state: State::Exited,
            source: Source::Pane,
            exit_code: p.exit_status,
            ..Status::unknown()
        };
    }
    if let Some(rec) = p
        .program_status
        .iter()
        .filter(|r| r.state.is_some())
        .max_by_key(|r| r.updated_ms.unwrap_or(0))
    {
        let state = match rec.state.as_deref() {
            Some("Working") => State::Working,
            Some("Blocked") => State::Blocked,
            Some("Done") => State::Done,
            Some("Error") => State::Error,
            Some("Idle") => State::Idle,
            _ => State::Unknown,
        };
        return Status {
            state,
            source: Source::Program,
            app: rec.app.clone(),
            message: rec.msg.clone(),
            updated_ms: rec.updated_ms,
            exit_code: None,
        };
    }
    if let Some(att) = &p.attention {
        let state = match att.status.as_deref() {
            Some("Done") => State::Done,
            Some("Failed") => State::Error,
            _ => State::Unknown,
        };
        if state != State::Unknown {
            let detail = att.detail.as_ref();
            return Status {
                state,
                source: Source::Attention,
                app: detail.and_then(|d| d.app.clone()),
                message: detail.and_then(|d| d.body.clone()),
                updated_ms: att.changed_ms.filter(|&ms| ms > 0),
                exit_code: None,
            };
        }
    }
    Status::unknown()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = format!(
            "{}/tests/fixtures/zellij/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(path).unwrap()
    }

    fn pane(panes: &[PaneInfo], id: u32) -> &PaneInfo {
        panes.iter().find(|p| p.pane == PaneId(id)).unwrap()
    }

    #[test]
    fn plugins_are_skipped() {
        let panes = parse_panes(&fixture("fresh")).unwrap();
        let ids: Vec<u32> = panes.iter().map(|p| p.pane.0).collect();
        assert_eq!(ids, [0, 1]);
    }

    #[test]
    fn fresh_pane_is_unknown() {
        let panes = parse_panes(&fixture("fresh")).unwrap();
        let p = pane(&panes, 1);
        assert_eq!(p.status, Status::unknown());
        assert_eq!(p.name, "fake");
        assert_eq!(p.tab_id, Some(0));
        assert!(p.floating);
        assert_eq!(p.cwd.as_deref(), Some("/work"));
        assert_eq!(p.command.as_deref(), Some("/bin/sh /work/fake.sh"));
    }

    #[test]
    fn working_from_program_status() {
        let panes = parse_panes(&fixture("working")).unwrap();
        let s = &pane(&panes, 1).status;
        assert_eq!(s.state, State::Working);
        assert_eq!(s.source, Source::Program);
        assert_eq!(s.updated_ms, Some(1_795_637_390_987));
    }

    #[test]
    fn done_from_program_status() {
        let panes = parse_panes(&fixture("done")).unwrap();
        insta::assert_json_snapshot!(pane(&panes, 1).status);
    }

    #[test]
    fn exited_wins_and_keeps_exit_code() {
        let panes = parse_panes(&fixture("exited")).unwrap();
        insta::assert_json_snapshot!(pane(&panes, 2).status);
    }

    #[test]
    fn blocked_carries_app_and_message() {
        let panes = parse_panes(&fixture("blocked")).unwrap();
        insta::assert_json_snapshot!(pane(&panes, 12).status);
    }

    /// Take pane 1 of the `done` fixture and change it.
    fn edited(edit: impl FnOnce(&mut serde_json::Value)) -> Status {
        let mut v: serde_json::Value = serde_json::from_str(&fixture("done")).unwrap();
        let p = v
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|p| p["id"] == 1 && p["is_plugin"] == false)
            .unwrap();
        edit(p);
        let panes = parse_panes(&v.to_string()).unwrap();
        pane(&panes, 1).status.clone()
    }

    #[test]
    fn newest_program_record_wins() {
        let s = edited(|p| {
            p["program_status"] = serde_json::json!([
                {"state": "Done", "updated_ms": 300, "app": "a"},
                {"state": "Working", "updated_ms": 500, "app": "b"},
                {"state": "Blocked", "updated_ms": 400}
            ]);
        });
        assert_eq!(s.state, State::Working);
        assert_eq!(s.app.as_deref(), Some("b"));
        assert_eq!(s.updated_ms, Some(500));
    }

    #[test]
    fn attention_fallback() {
        let cases = [
            ("Done", State::Done),
            ("Failed", State::Error),
            ("NewOutput", State::Unknown),
            ("NeedsYou", State::Unknown),
            ("Idle", State::Unknown),
        ];
        for (raw, want) in cases {
            let s = edited(|p| {
                p["program_status"] = serde_json::json!([]);
                p["attention"]["status"] = raw.into();
                p["attention"]["changed_ms"] = 42.into();
            });
            assert_eq!(s.state, want, "attention {raw}");
            let source = if want == State::Unknown {
                Source::None
            } else {
                Source::Attention
            };
            assert_eq!(s.source, source, "attention {raw}");
        }
    }

    #[test]
    fn exited_beats_program_status() {
        let s = edited(|p| {
            p["exited"] = true.into();
            p["exit_status"] = 0.into();
        });
        assert_eq!(s.state, State::Exited);
        assert_eq!(s.exit_code, Some(0));
    }

    #[test]
    fn bad_json_is_an_error() {
        assert!(parse_panes("Session 'x' not found").is_err());
    }
}
