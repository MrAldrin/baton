//! Integration tests: the real binary against a throwaway background Zellij
//! session `baton-test-<pid>-<test>`. The session is deleted on drop, also
//! when a test fails. A fake agent script reports OSC 7501 state.

use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use tempfile::TempDir;

const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake-agent.sh");

fn zellij() -> Command {
    let mut c = Command::new("zellij");
    for var in ["ZELLIJ", "ZELLIJ_SESSION_NAME", "ZELLIJ_PANE_ID"] {
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
    fn new(test: &str) -> Self {
        let name = format!("baton-test-{}-{test}", std::process::id());
        let status = zellij()
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
        s
    }

    /// Run baton; return the exit code and stdout.
    fn baton(&self, args: &[&str]) -> (i32, String) {
        let out = cargo_bin_cmd!("baton")
            .env_remove("ZELLIJ")
            .env_remove("ZELLIJ_SESSION_NAME")
            .env_remove("ZELLIJ_PANE_ID")
            .env("XDG_STATE_HOME", self.state.path())
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
        args.extend(["--", FAKE]);
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
    let s = Session::new("main");
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
    let s = Session::new("stale");
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
    let s = Session::new("blocked");
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
    let s = Session::new("exit");
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
    let s = Session::new("owner");
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
    let s = Session::new("agent");
    let cwd = s.work.path().to_str().unwrap();
    let (code, v) = s.json(&[
        "run", "--name", "c", "--cwd", cwd, "--tab-id", "0", "--agent", "claude", "--", FAKE,
    ]);
    assert_eq!(code, 0, "{v}");
    let id = v["session_id"].as_str().unwrap();
    assert_eq!(id.len(), 36);
    let (_, v) = s.json(&["list", "--owned"]);
    assert_eq!(v["panes"][0]["agent"], "claude");
    assert_eq!(v["panes"][0]["session_id"], id);
}

#[test]
fn missing_session() {
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
fn claude_hook_reports_state_to_the_pane() {
    let s = Session::new("hook");
    let cwd = s.work.path().to_str().unwrap();
    // Act like Claude Code: a process in the pane starts the hook with the
    // event on stdin and its stdout captured.
    let script = r#"printf '{"hook_event_name":"PermissionRequest","tool_name":"Bash"}' | "$0" hook claude > /dev/null; sleep 30"#;
    let (code, v) = s.json(&[
        "run",
        "--name",
        "h",
        "--cwd",
        cwd,
        "--tab-id",
        "0",
        "--",
        "sh",
        "-c",
        script,
        env!("CARGO_BIN_EXE_baton"),
    ]);
    assert_eq!(code, 0, "{v}");
    let pane = v["pane"].as_str().unwrap();
    let (code, v) = s.json(&["wait", pane, "--timeout", "10"]);
    assert_eq!(code, 2, "{v}");
    assert_eq!(v["state"], "blocked");
    assert_eq!(v["app"], "claude");
    assert_eq!(v["message"], "Bash");
}

#[test]
fn hook_is_silent_and_never_fails() {
    for input in ["not json", r#"{"hook_event_name":"Stop"}"#] {
        let out = cargo_bin_cmd!("baton")
            .args(["hook", "claude"])
            .write_stdin(input)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(out.stdout, b"");
    }
}

#[test]
fn agent_claude_adds_hook_settings() {
    let s = Session::new("csettings");
    let cwd = s.work.path().to_str().unwrap();
    let (code, v) = s.json(&[
        "run", "--name", "c", "--cwd", cwd, "--tab-id", "0", "--agent", "claude", "--", FAKE,
    ]);
    assert_eq!(code, 0, "{v}");
    let pane = v["pane"].as_str().unwrap();
    let rec = std::fs::read_to_string(s.state.path().join(format!("baton/{}~{pane}.json", s.name)))
        .unwrap();
    let rec: Value = serde_json::from_str(&rec).unwrap();
    let cmd = rec["command"].as_array().unwrap();
    assert_eq!(cmd[1], "--settings");
    let settings: Value = serde_json::from_str(cmd[2].as_str().unwrap()).unwrap();
    let hook = settings["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(hook.ends_with(" hook claude"), "{hook}");
    assert_eq!(cmd[3], "--session-id");
}
