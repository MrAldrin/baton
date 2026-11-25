//! The verbs. Each takes a [`Host`] and a [`Store`] and returns an
//! [`Outcome`]; `main` prints it.

use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde_json::{Value, json};

use crate::agent::{self, Agent};
use crate::host::{Host, PaneInfo, Spawn};
use crate::keys::key_bytes;
use crate::now_ms;
use crate::pane::PaneId;
use crate::state::{State, WAITABLE, wait_exit_code};
use crate::store::{Record, Store};

/// What a verb prints, and its exit code.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub output: Output,
    pub code: i32,
}

/// Printed on stdout.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    Json(Value),
    Text(String),
}

impl Outcome {
    fn json(v: Value) -> Self {
        Self {
            output: Output::Json(v),
            code: 0,
        }
    }
}

/// Shared inputs of all verbs.
pub struct Ctx<'a> {
    pub host: &'a dyn Host,
    pub store: &'a Store,
    /// The pane `baton` runs in, when it runs in the target session.
    pub caller_pane: Option<PaneId>,
}

impl Ctx<'_> {
    fn session(&self) -> &str {
        self.host.session()
    }

    fn record(&self, pane: PaneId) -> Result<Option<Record>> {
        self.store.load(self.session(), pane)
    }

    /// The record of an owned pane; `None` with `force` for other panes.
    fn owned_or_forced(&self, pane: PaneId, force: bool) -> Result<Option<Record>> {
        let rec = self.record(pane)?;
        if rec.is_none() && !force {
            bail!("pane {pane} was not started by baton; use --force to act on it anyway");
        }
        Ok(rec)
    }
}

/// Arguments of `run`.
#[derive(Debug, Clone)]
pub struct RunArgs {
    pub name: String,
    pub cwd: PathBuf,
    pub tab_id: Option<u64>,
    pub new_tab: Option<String>,
    pub agent: Option<Agent>,
    pub command: Vec<String>,
    /// This program, for the Claude Code state hooks.
    pub baton_exe: PathBuf,
}

/// `baton run`: start a program without focus and record ownership.
///
/// # Errors
/// Bad arguments, or the host fails.
pub fn run(ctx: &Ctx<'_>, args: RunArgs) -> Result<Outcome> {
    ensure!(
        !args.command.is_empty() || args.agent.is_some(),
        "give a command after --, or --agent"
    );
    let cwd = std::fs::canonicalize(&args.cwd)
        .with_context(|| format!("cannot use --cwd {}", args.cwd.display()))?;
    ensure!(cwd.is_dir(), "--cwd {} is not a folder", cwd.display());
    let (command, session_id) = match args.agent {
        Some(a) => agent::with_session_id(a, args.command, agent::new_uuid),
        None => (args.command, None),
    };
    let command = if args.agent == Some(Agent::Claude) {
        with_claude_hooks(command, &args.baton_exe)
    } else {
        command
    };

    let (pane, tab_id) = if let Some(tab_name) = &args.new_tab {
        let tab = ctx.host.new_tab(tab_name, &cwd)?;
        let pane = wait_for_shell(ctx.host, tab, Duration::from_secs(10))?;
        ctx.host.write_text(pane, &shell_join(&command))?;
        ctx.host.write_bytes(pane, b"\r")?;
        (pane, tab)
    } else {
        let tab = if let Some(t) = args.tab_id {
            t
        } else {
            let caller = ctx.caller_pane.ok_or_else(|| {
                anyhow!("give --tab-id or --new-tab (not inside this Zellij session)")
            })?;
            ctx.host
                .find_pane(caller)?
                .tab_id
                .ok_or_else(|| anyhow!("cannot find the tab of pane {caller}"))?
        };
        let pane = ctx.host.spawn_floating(&Spawn {
            name: &args.name,
            cwd: &cwd,
            tab_id: tab,
            command: &command,
        })?;
        (pane, tab)
    };

    ctx.store.save(&Record {
        session: ctx.session().to_owned(),
        pane,
        name: args.name,
        cwd,
        command,
        agent: args.agent,
        session_id: session_id.clone(),
        tab_id,
        started_ms: now_ms(),
        last_send_ms: None,
    })?;
    let mut out = json!({ "pane": pane, "tab_id": tab_id });
    if let Some(id) = session_id {
        out["session_id"] = id.into();
    }
    Ok(Outcome::json(out))
}

/// Add `--settings` with the state hooks after the program name, so
/// Claude Code reports state through `baton hook claude`.
fn with_claude_hooks(mut command: Vec<String>, baton_exe: &std::path::Path) -> Vec<String> {
    let hook = shell_join(&[
        baton_exe.to_string_lossy().into_owned(),
        "hook".into(),
        "claude".into(),
    ]);
    let at = 1.min(command.len());
    command.splice(
        at..at,
        ["--settings".to_owned(), crate::hook::claude_settings(&hook)],
    );
    command
}

/// Wait until the new tab has a terminal pane with a running shell.
fn wait_for_shell(host: &dyn Host, tab: u64, limit: Duration) -> Result<PaneId> {
    let start = Instant::now();
    loop {
        let found = host
            .list_panes()?
            .into_iter()
            .find(|p| p.tab_id == Some(tab) && p.command.is_some());
        if let Some(p) = found {
            // Give the shell a moment to print its prompt and read input.
            sleep(Duration::from_millis(300));
            return Ok(p.pane);
        }
        if start.elapsed() > limit {
            bail!("tab {tab} has no shell after {}s", limit.as_secs());
        }
        sleep(Duration::from_millis(200));
    }
}

/// Join words for a POSIX shell, quoting where needed.
///
/// ```
/// use baton::commands::shell_join;
/// assert_eq!(shell_join(&["pi".into(), "it's ok".into()]), r#"pi 'it'\''s ok'"#);
/// ```
#[must_use]
pub fn shell_join(words: &[String]) -> String {
    words
        .iter()
        .map(|w| {
            let plain = !w.is_empty()
                && w.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_./=:,+@%".contains(&b));
            if plain {
                w.clone()
            } else {
                format!("'{}'", w.replace('\'', r"'\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `baton send`: type text and submit it.
///
/// # Errors
/// The pane is not owned or is busy (without `force`), or the host fails.
pub fn send(ctx: &Ctx<'_>, pane: PaneId, text: &str, enter: bool, force: bool) -> Result<Outcome> {
    let info = ctx.host.find_pane(pane)?;
    let rec = ctx.owned_or_forced(pane, force)?;
    if !force && matches!(info.status.state, State::Working | State::Blocked) {
        bail!(
            "pane {pane} is {}; use --force to send anyway",
            state_name(info.status.state)
        );
    }
    let sent = now_ms();
    if let Some(mut r) = rec {
        r.last_send_ms = Some(sent);
        ctx.store.save(&r)?;
    }
    ctx.host.write_text(pane, text)?;
    if enter {
        ctx.host.write_bytes(pane, b"\r")?;
    }
    Ok(Outcome::json(json!({ "pane": pane, "sent_ms": sent })))
}

/// `baton keys`: send named keys.
///
/// # Errors
/// An unknown key, a pane that is not owned (without `force`), or the host
/// fails.
pub fn keys(ctx: &Ctx<'_>, pane: PaneId, names: &[String], force: bool) -> Result<Outcome> {
    let bytes = names
        .iter()
        .map(|k| key_bytes(k))
        .collect::<Result<Vec<_>>>()?;
    ctx.host.find_pane(pane)?;
    let rec = ctx.owned_or_forced(pane, force)?;
    // Keys can start a turn (Enter, or an answer to a prompt): record the
    // time like `send`, so `wait` ignores the old state.
    if let Some(mut r) = rec {
        r.last_send_ms = Some(now_ms());
        ctx.store.save(&r)?;
    }
    for b in bytes {
        ctx.host.write_bytes(pane, b)?;
    }
    Ok(Outcome::json(json!({ "pane": pane, "keys": names })))
}

/// `baton read`: the pane's text.
///
/// # Errors
/// The host fails.
pub fn read(
    ctx: &Ctx<'_>,
    pane: PaneId,
    full: bool,
    lines: Option<usize>,
    as_json: bool,
) -> Result<Outcome> {
    ctx.host.find_pane(pane)?;
    let raw = ctx.host.dump_screen(pane, full)?;
    let text = last_lines(&raw, lines);
    Ok(if as_json {
        Outcome::json(json!({ "pane": pane, "text": text }))
    } else {
        Outcome {
            output: Output::Text(text),
            code: 0,
        }
    })
}

/// Drop trailing blank lines, then keep the last `n` lines.
fn last_lines(raw: &str, n: Option<usize>) -> String {
    let mut lines: Vec<&str> = raw.lines().map(str::trim_end).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let skip = n.map_or(0, |n| lines.len().saturating_sub(n));
    let mut out = lines[skip..].join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// `baton status`.
///
/// # Errors
/// The pane does not exist, or the host fails.
pub fn status(ctx: &Ctx<'_>, pane: PaneId) -> Result<Outcome> {
    let info = ctx.host.find_pane(pane)?;
    Ok(Outcome::json(serde_json::to_value(info.status)?))
}

/// `baton wait`: poll until a state in `until` is reached after the last
/// send. Exit codes: 0 done or idle, 1 error or exited, 2 blocked, 124
/// timeout.
///
/// # Errors
/// A state in `until` that `wait` does not accept, or the host fails.
pub fn wait(
    ctx: &Ctx<'_>,
    pane: PaneId,
    until: &[State],
    timeout: Option<Duration>,
    poll: Duration,
) -> Result<Outcome> {
    ensure!(!until.is_empty(), "--until needs at least one state");
    if let Some(bad) = until.iter().find(|s| !WAITABLE.contains(s)) {
        bail!(
            "cannot wait for {}: use done, idle, blocked, error or exited",
            state_name(*bad)
        );
    }
    let sent = ctx.record(pane)?.and_then(|r| r.last_send_ms);
    let start = Instant::now();
    loop {
        let status = ctx.host.find_pane(pane)?.status;
        if status.satisfies(until, sent) {
            let code = wait_exit_code(status.state);
            return Ok(Outcome {
                output: Output::Json(serde_json::to_value(status)?),
                code,
            });
        }
        if timeout.is_some_and(|t| start.elapsed() >= t) {
            let mut v = serde_json::to_value(status)?;
            v["timed_out"] = true.into();
            return Ok(Outcome {
                output: Output::Json(v),
                code: 124,
            });
        }
        sleep(poll);
    }
}

/// `baton list`: terminal panes with name, command, cwd and state.
///
/// # Errors
/// The host fails or a record cannot be read.
pub fn list(ctx: &Ctx<'_>, owned_only: bool) -> Result<Outcome> {
    let mut rows = Vec::new();
    for p in ctx.host.list_panes()? {
        let rec = ctx.record(p.pane)?;
        if owned_only && rec.is_none() {
            continue;
        }
        rows.push(list_row(&p, rec.as_ref()));
    }
    Ok(Outcome::json(json!({ "panes": rows })))
}

fn list_row(p: &PaneInfo, rec: Option<&Record>) -> Value {
    json!({
        "pane": p.pane,
        "name": p.name,
        "command": p.command,
        "cwd": p.cwd,
        "tab_id": p.tab_id,
        "state": p.status.state,
        "owned": rec.is_some(),
        "agent": rec.and_then(|r| r.agent),
        "session_id": rec.and_then(|r| r.session_id.clone()),
    })
}

/// `baton close`: close a pane `baton` started (others with `force`) and
/// remove its record.
///
/// # Errors
/// The pane does not exist or is not owned (without `force`), or the host
/// fails.
pub fn close(ctx: &Ctx<'_>, pane: PaneId, force: bool) -> Result<Outcome> {
    if let Err(e) = ctx.host.find_pane(pane) {
        // The pane is gone: drop a stale record, then report.
        ctx.store.remove(ctx.session(), pane)?;
        return Err(e);
    }
    ctx.owned_or_forced(pane, force)?;
    ctx.host.close_pane(pane)?;
    ctx.store.remove(ctx.session(), pane)?;
    Ok(Outcome::json(json!({ "pane": pane, "closed": true })))
}

fn state_name(s: State) -> String {
    serde_json::to_value(s)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::Path;

    use super::*;
    use crate::state::{Source, Status};

    /// A host that returns a scripted status per `list_panes` call and
    /// records writes.
    struct FakeHost {
        statuses: RefCell<Vec<Status>>,
        writes: RefCell<Vec<Vec<u8>>>,
    }

    impl FakeHost {
        fn new(statuses: Vec<Status>) -> Self {
            Self {
                statuses: RefCell::new(statuses),
                writes: RefCell::new(Vec::new()),
            }
        }
    }

    impl Host for FakeHost {
        fn session(&self) -> &'static str {
            "fake"
        }
        fn list_panes(&self) -> Result<Vec<PaneInfo>> {
            let mut s = self.statuses.borrow_mut();
            let status = if s.len() > 1 {
                s.remove(0)
            } else {
                s[0].clone()
            };
            Ok(vec![PaneInfo {
                pane: PaneId(1),
                name: "a".into(),
                command: Some("sh".into()),
                cwd: None,
                tab_id: Some(0),
                floating: true,
                status,
            }])
        }
        fn spawn_floating(&self, _: &Spawn<'_>) -> Result<PaneId> {
            Ok(PaneId(1))
        }
        fn new_tab(&self, _: &str, _: &Path) -> Result<u64> {
            Ok(1)
        }
        fn write_text(&self, _: PaneId, text: &str) -> Result<()> {
            self.writes.borrow_mut().push(text.as_bytes().to_vec());
            Ok(())
        }
        fn write_bytes(&self, _: PaneId, bytes: &[u8]) -> Result<()> {
            self.writes.borrow_mut().push(bytes.to_vec());
            Ok(())
        }
        fn dump_screen(&self, _: PaneId, _: bool) -> Result<String> {
            Ok("a\nb\nc\n\n\n".into())
        }
        fn close_pane(&self, _: PaneId) -> Result<()> {
            Ok(())
        }
    }

    fn st(state: State, ms: u64) -> Status {
        Status {
            state,
            source: Source::Program,
            updated_ms: Some(ms),
            ..Status::unknown()
        }
    }

    fn owned(store: &Store, last_send_ms: Option<u64>) {
        store
            .save(&Record {
                session: "fake".into(),
                pane: PaneId(1),
                name: "a".into(),
                cwd: "/".into(),
                command: vec!["sh".into()],
                agent: None,
                session_id: None,
                tab_id: 0,
                started_ms: 0,
                last_send_ms,
            })
            .unwrap();
    }

    const UNTIL: [State; 4] = [State::Done, State::Blocked, State::Error, State::Exited];
    const POLL: Duration = Duration::from_millis(1);

    #[test]
    fn wait_skips_stale_done() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::at(tmp.path().into());
        owned(&store, Some(1000));
        let host = FakeHost::new(vec![
            st(State::Done, 900),
            st(State::Working, 1001),
            st(State::Done, 1002),
        ]);
        let ctx = Ctx {
            host: &host,
            store: &store,
            caller_pane: None,
        };
        let out = wait(&ctx, PaneId(1), &UNTIL, None, POLL).unwrap();
        assert_eq!(out.code, 0);
        let Output::Json(v) = out.output else {
            panic!()
        };
        assert_eq!(v["updated_ms"], 1002);
        assert_eq!(host.statuses.borrow().len(), 1);
    }

    #[test]
    fn wait_times_out_on_stale_done() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::at(tmp.path().into());
        owned(&store, Some(1000));
        let host = FakeHost::new(vec![st(State::Done, 900)]);
        let ctx = Ctx {
            host: &host,
            store: &store,
            caller_pane: None,
        };
        let out = wait(
            &ctx,
            PaneId(1),
            &UNTIL,
            Some(Duration::from_millis(20)),
            POLL,
        )
        .unwrap();
        assert_eq!(out.code, 124);
        let Output::Json(v) = out.output else {
            panic!()
        };
        assert_eq!(v["timed_out"], true);
        assert_eq!(v["state"], "done");
    }

    #[test]
    fn wait_blocked_is_2_and_rejects_working() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::at(tmp.path().into());
        let host = FakeHost::new(vec![st(State::Blocked, 5)]);
        let ctx = Ctx {
            host: &host,
            store: &store,
            caller_pane: None,
        };
        assert_eq!(wait(&ctx, PaneId(1), &UNTIL, None, POLL).unwrap().code, 2);
        assert!(wait(&ctx, PaneId(1), &[State::Working], None, POLL).is_err());
    }

    #[test]
    fn send_refuses_busy_and_foreign_panes() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::at(tmp.path().into());
        let host = FakeHost::new(vec![st(State::Done, 5)]);
        let ctx = Ctx {
            host: &host,
            store: &store,
            caller_pane: None,
        };
        let err = send(&ctx, PaneId(1), "hi", true, false).unwrap_err();
        assert!(err.to_string().contains("not started by baton"), "{err}");
        owned(&store, None);
        *host.statuses.borrow_mut() = vec![st(State::Blocked, 5)];
        let err = send(&ctx, PaneId(1), "hi", true, false).unwrap_err();
        assert!(err.to_string().contains("is blocked"), "{err}");
        assert!(host.writes.borrow().is_empty());
        send(&ctx, PaneId(1), "hi", true, true).unwrap();
        assert_eq!(*host.writes.borrow(), [b"hi".to_vec(), b"\r".to_vec()]);
        assert!(
            store
                .load("fake", PaneId(1))
                .unwrap()
                .unwrap()
                .last_send_ms
                .is_some()
        );
    }

    #[test]
    fn read_trims_and_limits_lines() {
        assert_eq!(last_lines("a\nb\nc\n\n\n", Some(2)), "b\nc\n");
        assert_eq!(last_lines("a  \nb\n", None), "a\nb\n");
        assert_eq!(last_lines("\n\n", Some(3)), "");
    }

    #[test]
    fn shell_join_quotes() {
        let w = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            shell_join(&w(&["pi", "--session-id", "a-1"])),
            "pi --session-id a-1"
        );
        assert_eq!(shell_join(&w(&["echo", "", "a b"])), "echo '' 'a b'");
    }
}
