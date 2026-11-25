//! Fake agent for the integration tests. It reports state with OSC 7501,
//! as Pi does, so no model is needed. It replaces the old `sh` script so
//! the tests also run on Windows.
//!
//! Each input line is one turn:
//! - `block`: asks a question (blocked); the next line answers it
//! - `fail`: error
//! - `exit N`: exits with code N
//! - `slow`: works for 3 s
//! - other: prints `reply: <line>` and is done
//!
//! `fake_agent env FILE [CODE]` writes its environment (`KEY=value` lines)
//! to FILE, then exits with CODE, or sleeps 30 s without CODE.

use std::fmt::Write as _;
use std::io::{BufRead, Write};
use std::thread::sleep;
use std::time::Duration;

fn osc(state: &str) {
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "\x1b]7501;state={state}:app=fake\x1b\\");
    let _ = out.flush();
}

fn say(text: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{text}");
    let _ = out.flush();
}

fn dump_env(file: &str, code: Option<i32>) -> ! {
    let mut text = String::new();
    for (k, v) in std::env::vars_os() {
        let _ = writeln!(text, "{}={}", k.to_string_lossy(), v.to_string_lossy());
    }
    std::fs::write(file, text).expect("cannot write env file");
    if code.is_none() {
        sleep(Duration::from_secs(30));
    }
    std::process::exit(code.unwrap_or(0));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("env") {
        let code = args.get(2).map(|c| c.parse().expect("bad exit code"));
        dump_env(&args[1], code);
    }

    osc("idle");
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    while let Some(Ok(line)) = lines.next() {
        let line = line.trim_end_matches(['\r', '\n']);
        osc("working");
        match line {
            "block" => {
                say("approve? 1 yes 2 no");
                osc("blocked");
                let answer = lines.next().and_then(Result::ok).unwrap_or_default();
                osc("working");
                say(&format!("answer: {}", answer.trim_end()));
            }
            "fail" => {
                say("failed");
                osc("error");
                continue;
            }
            "slow" => sleep(Duration::from_secs(3)),
            _ => {
                if let Some(code) = line.strip_prefix("exit ") {
                    std::process::exit(code.trim().parse().unwrap_or(1));
                }
                say(&format!("reply: {line}"));
            }
        }
        sleep(Duration::from_millis(300));
        osc("done");
    }
}
