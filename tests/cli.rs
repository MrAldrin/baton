//! Integration tests: the real binary against a throwaway background Zellij
//! session `baton-test-<pid>-<test>`. The session is deleted on drop, also
//! when a test fails. A fake agent (`examples/fake_agent.rs`) reports OSC
//! 7501 state.
//!
//! Tests that need Zellij 0.46.1 skip themselves when it is not on `PATH`,
//! unless `BATON_REQUIRE_ZELLIJ` is set (CI sets it where Zellij is
//! installed); then a missing Zellij fails them.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::thread::sleep;
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin_cmd;
use baton::commands::CLAUDE_HOST_VARS;
use serde_json::Value;
use tempfile::TempDir;

/// The fake agent, built by Cargo as an example in the `examples` folder
/// next to the `baton` binary (`target/<profile>/examples/`).
fn fake() -> String {
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_baton"));
    let dir = exe.parent().unwrap().join("examples");
    let path: PathBuf = dir.join(format!("fake_agent{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.exists(),
        "{} is missing: run `cargo build --examples`",
        path.display()
    );
    path.to_string_lossy().into_owned()
}

/// Whether Zellij 0.46.1 is on `PATH`. Without it, tests skip, or fail when
/// `BATON_REQUIRE_ZELLIJ` is set.
fn have_zellij() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    let found = *FOUND.get_or_init(|| {
        Command::new("zellij")
            .arg("--version")
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("0.46.1"))
    });
    if !found {
        assert!(
            std::env::var_os("BATON_REQUIRE_ZELLIJ").is_none(),
            "BATON_REQUIRE_ZELLIJ is set, but zellij 0.46.1 is not on PATH"
        );
        eprintln!("skipped: zellij 0.46.1 is not on PATH");
    }
    found
}

/// `zellij` without the caller's Zellij and Claude Code host variables, so
/// test sessions start with the same clean environment as `baton run`.
fn zellij() -> Command {
    let mut c = Command::new("zellij");
    for var in ["ZELLIJ", "ZELLIJ_SESSION_NAME", "ZELLIJ_PANE_ID"]
        .into_iter()
        .chain(CLAUDE_HOST_VARS)
    {
        c.env_remove(var);
    }
    c
}

struct Session {
    name: String,
    state: TempDir,
    work: TempDir,
}

impl Session {
    /// A new session, or `None` when Zellij is missing (the test skips).
    fn new(test: &str) -> Option<Self> {
        Self::with_env(test, &[])
    }

    /// A session whose server (and so its panes) has extra variables.
    fn with_env(test: &str, vars: &[(&str, &str)]) -> Option<Self> {
        if !have_zellij() {
            return None;
        }
        let name = format!("baton-test-{}-{test}", std::process::id());
        let status = zellij()
            .envs(vars.iter().copied())
            .args(["attach", "--create-background", &name])
            .status()
            .expect("zellij must be installed");
        assert!(status.success(), "cannot create session {name}");
        let s = Self {
            name,
            state: tempfile::tempdir().unwrap(),
            work: tempfile::tempdir().unwrap(),
        };
        // Wait until the session answers.
        let start = Instant::now();
        while s.baton(&["list"]).0 != 0 {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "session not ready"
            );
            sleep(Duration::from_millis(100));
        }
        Some(s)
    }

    /// Run baton; return the exit code and stdout.
    fn baton(&self, args: &[&str]) -> (i32, String) {
        let out = cargo_bin_cmd!("baton")
            .env_remove("ZELLIJ")
            .env_remove("ZELLIJ_SESSION_NAME")
            .env_remove("ZELLIJ_PANE_ID")
            .env("XDG_STATE_HOME", self.state.path())
            .env("LOCALAPPDATA", self.state.path())
            .arg("--session")
            .arg(&self.name)
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    }

    /// Run baton and parse its JSON output.
    fn json(&self, args: &[&str]) -> (i32, Value) {
        let (code, out) = self.baton(args);
        let v = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out:?}"));
        (code, v)
    }

    /// Start the fake agent; wait until it reports idle.
    fn start_fake(&self, extra: &[&str]) -> String {
        let cwd = self.work.path().to_str().unwrap();
        let mut args = vec!["run", "--name", "fake", "--cwd", cwd, "--tab-id", "0"];
        args.extend(extra);
        let fake = fake();
        args.extend(["--", &fake]);
        let (code, v) = self.json(&args);
        assert_eq!(code, 0, "{v}");
        let pane = v["pane"].as_str().unwrap().to_owned();
        let (code, v) = self.json(&["wait", &pane, "--until", "idle", "--timeout", "10"]);
        assert_eq!(code, 0, "{v}");
        pane
    }

    fn records(&self) -> usize {
        std::fs::read_dir(self.state.path().join("baton")).map_or(0, Iterator::count)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = zellij()
            .args(["delete-session", "--force", &self.name])
            .output();
    }
}

#[test]
fn run_send_wait_read_list_close() {
    let Some(s) = Session::new("main") else {
        return;
    };
    let pane = s.start_fake(&[]);
    assert!(pane.starts_with("terminal_"));
    assert_eq!(s.records(), 1);

    let (code, v) = s.json(&["status", &pane]);
    assert_eq!(code, 0);
    assert_eq!(v["state"], "idle");
    assert_eq!(v["source"], "program");
    assert_eq!(v["app"], "fake");

    let (code, v) = s.json(&["send", &pane, "hello"]);
    assert_eq!(code, 0, "{v}");
    let (code, v) = s.json(&["wait", &pane, "--timeout", "10"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["state"], "done");

    let (code, text) = s.baton(&["read", &pane]);
    assert_eq!(code, 0);
    assert!(text.contains("reply: hello"), "{text:?}");
    let (_, v) = s.json(&["read", &pane, "--json", "--lines", "1"]);
    assert_eq!(v["text"], "reply: hello\n");

    let (code, v) = s.json(&["list", "--owned"]);
    assert_eq!(code, 0);
    let panes = v["panes"].as_array().unwrap();
    assert_eq!(panes.len(), 1);
    assert_eq!(panes[0]["pane"], pane.as_str());
    assert_eq!(panes[0]["name"], "fake");
    assert_eq!(panes[0]["state"], "done");
    assert_eq!(panes[0]["owned"], true);

    let (code, v) = s.json(&["close", &pane]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(s.records(), 0);
    let (_, v) = s.json(&["list"]);
    assert!(
        v["panes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["pane"] != pane.as_str())
    );
}

#[test]
fn stale_done_and_timeout() {
    let Some(s) = Session::new("stale") else {
        return;
    };
    let pane = s.start_fake(&[]);
    s.json(&["send", &pane, "one"]);
    assert_eq!(s.json(&["wait", &pane, "--timeout", "10"]).0, 0);

    // A send without Enter records the send time but starts no turn, so
    // the old Done is stale and wait must time out.
    s.json(&["send", &pane, "two", "--no-enter"]);
    let (code, v) = s.json(&["wait", &pane, "--timeout", "1.5"]);
    assert_eq!(code, 124, "{v}");
    assert_eq!(v["state"], "done");
    assert_eq!(v["timed_out"], true);

    // Enter starts the turn; now a new Done counts.
    assert_eq!(s.json(&["keys", &pane, "enter"]).0, 0);
    let (code, v) = s.json(&["wait", &pane, "--timeout", "10"]);
    assert_eq!(code, 0, "{v}");
    let (_, text) = s.baton(&["read", &pane]);
    assert!(text.contains("reply: two"), "{text:?}");
}

#[test]
fn blocked_then_keys() {
    let Some(s) = Session::new("blocked") else {
        return;
    };
    let pane = s.start_fake(&[]);
    s.json(&["send", &pane, "block"]);
    let (code, v) = s.json(&["wait", &pane, "--timeout", "10"]);
    assert_eq!(code, 2, "{v}");
    assert_eq!(v["state"], "blocked");

    let (code, v) = s.json(&["send", &pane, "nope"]);
    assert_eq!(code, 3);
    assert!(v["error"]["message"].as_str().unwrap().contains("blocked"));

    // keys records the time, so the old Blocked is stale and wait goes on
    // to the new Done.
    assert_eq!(s.json(&["keys", &pane, "1", "enter"]).0, 0);
    let (code, v) = s.json(&["wait", &pane, "--timeout", "10"]);
    assert_eq!(code, 0, "{v}");
    let (_, text) = s.baton(&["read", &pane]);
    assert!(text.contains("answer: 1"), "{text:?}");
}

#[test]
fn error_and_exit() {
    let Some(s) = Session::new("exit") else {
        return;
    };
    let pane = s.start_fake(&[]);
    s.json(&["send", &pane, "fail"]);
    let (code, v) = s.json(&["wait", &pane, "--timeout", "10"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["state"], "error");

    s.json(&["send", &pane, "exit 4"]);
    let (code, v) = s.json(&["wait", &pane, "--timeout", "10"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["state"], "exited");
    assert_eq!(v["exit_code"], 4);
}

#[test]
fn ownership_refusal_and_errors() {
    let Some(s) = Session::new("owner") else {
        return;
    };
    // terminal_0 is the session's own shell; baton did not start it.
    let (code, v) = s.json(&["list"]);
    assert_eq!(code, 0);
    let first = &v["panes"][0];
    assert_eq!(first["pane"], "terminal_0");
    assert_eq!(first["owned"], false);
    assert_eq!(
        s.json(&["list", "--owned"]).1["panes"],
        serde_json::json!([])
    );
    assert_eq!(s.json(&["status", "0"]).0, 0);

    for args in [
        &["send", "terminal_0", "echo hi"][..],
        &["keys", "terminal_0", "enter"],
        &["close", "terminal_0"],
    ] {
        let (code, v) = s.json(args);
        assert_eq!(code, 3, "{args:?}");
        let msg = v["error"]["message"].as_str().unwrap();
        assert!(msg.contains("not started by baton"), "{msg}");
    }

    // A forced send works and keeps the send time, but the pane stays
    // not owned.
    let (code, v) = s.json(&["send", "terminal_0", "true", "--force"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(s.records(), 1);
    let (_, v) = s.json(&["list"]);
    assert_eq!(v["panes"][0]["owned"], false);
    assert_eq!(s.json(&["close", "terminal_0"]).0, 3);

    let (code, v) = s.json(&["status", "terminal_99"]);
    assert_eq!(code, 3);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not found")
    );

    let (code, v) = s.json(&["keys", "terminal_0", "f1"]);
    assert_eq!(code, 3);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown key")
    );

    let (code, v) = s.json(&["wait", "terminal_0", "--until", "working"]);
    assert_eq!(code, 3, "{v}");

    // A bad argument is also an error in JSON with exit code 3.
    let (code, v) = s.json(&["status"]);
    assert_eq!(code, 3);
    assert!(v["error"]["message"].is_string());
}

#[test]
fn agent_sets_session_id() {
    let Some(s) = Session::new("agent") else {
        return;
    };
    let cwd = s.work.path().to_str().unwrap();
    let fake = fake();
    let (code, v) = s.json(&[
        "run", "--name", "c", "--cwd", cwd, "--tab-id", "0", "--agent", "claude", "--", &fake,
    ]);
    assert_eq!(code, 0, "{v}");
    let id = v["session_id"].as_str().unwrap();
    assert_eq!(id.len(), 36);
    let (_, v) = s.json(&["list", "--owned"]);
    assert_eq!(v["panes"][0]["agent"], "claude");
    assert_eq!(v["panes"][0]["session_id"], id);
    let pane = v["panes"][0]["pane"].as_str().unwrap();
    let rec = std::fs::read_to_string(s.state.path().join(format!("baton/{}~{pane}.json", s.name)))
        .unwrap();
    let rec: Value = serde_json::from_str(&rec).unwrap();
    let cmd: Vec<&str> = rec["command"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    // The OS launcher comes first: `env -u …` on Unix, `baton exec --` on
    // Windows.
    if cfg!(unix) {
        assert_eq!(cmd[0], "env");
    } else {
        assert_eq!(cmd[1..3], ["exec", "--"]);
    }
    let at = cmd.iter().position(|a| *a == fake).unwrap();
    assert_eq!(cmd[at + 1..], ["--session-id", id]);
}

#[test]
fn missing_session() {
    if !have_zellij() {
        return;
    }
    let out = cargo_bin_cmd!("baton")
        .env_remove("ZELLIJ_SESSION_NAME")
        .args(["--session", "baton-test-no-such-session", "list"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not found")
    );

    let out = cargo_bin_cmd!("baton")
        .env_remove("ZELLIJ_SESSION_NAME")
        .arg("list")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn run_unsets_claude_host_vars() {
    let Some(s) = Session::with_env(
        "hostvars",
        &[
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_DISABLE_TERMINAL_TITLE", "1"),
            ("BATON_TEST_KEEP", "1"),
        ],
    ) else {
        return;
    };
    let cwd = s.work.path().to_str().unwrap();
    let file = s.work.path().join("env.txt");
    let fake = fake();
    let (code, v) = s.json(&[
        "run",
        "--name",
        "env",
        "--cwd",
        cwd,
        "--tab-id",
        "0",
        "--",
        &fake,
        "env",
        file.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{v}");
    let start = Instant::now();
    let text = loop {
        if let Ok(t) = std::fs::read_to_string(&file)
            && t.contains("BATON_TEST_KEEP")
        {
            break t;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "no env.txt");
        sleep(Duration::from_millis(100));
    };
    for var in CLAUDE_HOST_VARS {
        assert!(!text.contains(&format!("{var}=")), "{var} leaked");
    }
}

/// The launcher `run` uses on Windows, checked on every OS without Zellij.
#[test]
fn exec_unsets_claude_host_vars_and_keeps_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("env.txt");
    let out = cargo_bin_cmd!("baton")
        .env("CLAUDECODE", "1")
        .env("CLAUDE_CODE_DISABLE_TERMINAL_TITLE", "1")
        .env("BATON_TEST_KEEP", "1")
        .args(["exec", "--", &fake(), "env", file.to_str().unwrap(), "7"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(7));
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("BATON_TEST_KEEP=1"), "{text}");
    for var in CLAUDE_HOST_VARS {
        assert!(!text.contains(&format!("{var}=")), "{var} leaked");
    }
}
