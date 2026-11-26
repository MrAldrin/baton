//! How `run` starts a program on each OS.
//!
//! Zellij panes inherit the environment of the Zellij server, not of the
//! `baton` process that asks for the pane. So `baton` cannot remove
//! [`CLAUDE_HOST_VARS`] with [`Command::env_remove`] on the pane itself; it
//! must put a small launcher in front of the command, inside the pane:
//!
//! - Unix: `env -u VAR … <cmd>` ([`without_host_vars`]), as in v1.
//! - Windows: `<baton.exe> exec -- <cmd>`. The hidden `exec` verb removes
//!   the variables with [`Command::env_remove`], starts the program and
//!   returns its exit code ([`exec`]). It finds `.cmd`/`.bat` shims
//!   through `PATHEXT` ([`find_program`]).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::agent::Agent;
use crate::commands::{CLAUDE_HOST_VARS, without_host_vars};

/// Variables an agent gets when they are not set yet in the pane.
///
/// Pi reports its state with OSC 7501 only after the terminal answers its
/// feature query (`ESC ] 7501 ; ? ESC \`). On Windows the answer does not
/// pass back through `ConPTY`, so Pi stays silent and `wait` times out.
/// `PI_PROGRAM_STATUS=1` makes Pi skip the query; `baton` only runs it in
/// Zellij, which reads OSC 7501.
#[must_use]
pub fn default_env(agent: Option<Agent>) -> &'static [&'static str] {
    match agent {
        Some(Agent::Pi) => &["PI_PROGRAM_STATUS=1"],
        _ => &[],
    }
}

/// The command that a pane runs for `command`: the OS launcher in front of
/// it, so the program starts without [`CLAUDE_HOST_VARS`]. On Windows the
/// launcher also sets [`default_env`] for `agent`.
///
/// # Errors
/// On Windows, the path of the running `baton` is not known.
pub fn clean_command(command: Vec<String>, agent: Option<Agent>) -> Result<Vec<String>> {
    if cfg!(unix) {
        Ok(without_host_vars(command))
    } else {
        let exe = std::env::current_exe().context("cannot find the baton program")?;
        let mut out = vec![
            plain_path(&exe).to_string_lossy().into_owned(),
            "exec".to_owned(),
        ];
        for pair in default_env(agent) {
            out.push("--default-env".to_owned());
            out.push((*pair).to_owned());
        }
        out.push("--".to_owned());
        out.extend(command);
        Ok(out)
    }
}

/// `baton exec`: start `command` without [`CLAUDE_HOST_VARS`], wait for it
/// and return its exit code. On Unix the program replaces `baton`.
///
/// # Errors
/// The command is empty or cannot start.
pub fn exec(command: &[String], default_env: &[String]) -> Result<i32> {
    let (program, args) = command.split_first().context("give a command after --")?;
    // Rust runs a `.bat`/`.cmd` path through cmd.exe with safe quoting.
    let mut cmd = Command::new(resolve(program));
    cmd.args(args);
    for var in CLAUDE_HOST_VARS {
        cmd.env_remove(var);
    }
    for (key, value) in missing_env(default_env, |k| std::env::var_os(k).is_some()) {
        cmd.env(key, value);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        Err(err).with_context(|| format!("cannot start {program}"))
    }
    #[cfg(not(unix))]
    {
        #[cfg(windows)]
        console::ignore_ctrl_c();
        let status = cmd
            .status()
            .with_context(|| format!("cannot start {program}"))?;
        Ok(status.code().unwrap_or(1))
    }
}

/// The `KEY=VALUE` pairs whose `KEY` is not set (`is_set` says false).
/// A value the user set wins. Pairs without `=` are skipped.
fn missing_env(pairs: &[String], is_set: impl Fn(&str) -> bool) -> Vec<(&str, &str)> {
    pairs
        .iter()
        .filter_map(|p| p.split_once('='))
        .filter(|(k, _)| !k.is_empty() && !is_set(k))
        .collect()
}

#[cfg(windows)]
mod console {
    type Handler = unsafe extern "system" fn(u32) -> i32;

    unsafe extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<Handler>, add: i32) -> i32;
    }

    unsafe extern "system" fn handled(_: u32) -> i32 {
        1
    }

    /// Ctrl-C goes to every process on the console. `baton exec` must not
    /// die before its child, or the pane shows "exited" while the program
    /// still runs. A handler (unlike `SetConsoleCtrlHandler(NULL, TRUE)`)
    /// is not inherited, so the child still gets Ctrl-C.
    pub fn ignore_ctrl_c() {
        // SAFETY: registers a static function that only returns TRUE.
        unsafe {
            SetConsoleCtrlHandler(Some(handled), 1);
        }
    }
}

/// `path` without the Windows `\\?\` prefix that [`std::fs::canonicalize`]
/// adds; cmd.exe cannot use such a folder. Other paths stay as they are.
///
/// ```
/// use std::path::Path;
/// use baton::launch::plain_path;
/// assert_eq!(plain_path(Path::new(r"\\?\C:\work")), Path::new(r"C:\work"));
/// assert_eq!(plain_path(Path::new("/work")), Path::new("/work"));
/// ```
#[must_use]
pub fn plain_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

/// The program file that Windows would run for `program`, found the way
/// cmd.exe does: in `path` (the `PATH` folders), with each extension in
/// `pathext` (such as `.COM;.EXE;.BAT;.CMD`). [`Command`] alone only adds
/// `.exe`, so npm shims such as `pi.cmd` are not found. A name with a
/// folder is tried as it is first, then with each extension. Files without
/// an extension (POSIX shims next to the `.cmd`) are never picked.
/// Returns `None` when nothing matches.
#[must_use]
pub fn find_program(program: &str, path: &std::ffi::OsStr, pathext: &str) -> Option<PathBuf> {
    let exts: Vec<&str> = pathext
        .split(';')
        .map(str::trim)
        .filter(|e| e.starts_with('.') && e.len() > 1)
        .collect();
    let has_ext = |p: &Path| {
        p.extension().is_some_and(|e| {
            let e = e.to_string_lossy();
            exts.iter().any(|x| x[1..].eq_ignore_ascii_case(&e))
        })
    };
    let try_base = |base: &Path| -> Option<PathBuf> {
        if has_ext(base) && base.is_file() {
            return Some(base.to_path_buf());
        }
        // Windows folders ignore case; also try lowercase for other file
        // systems (and the tests on Linux).
        exts.iter()
            .flat_map(|ext| [(*ext).to_owned(), ext.to_ascii_lowercase()])
            .find_map(|ext| {
                let mut name = base.as_os_str().to_owned();
                name.push(ext);
                let candidate = PathBuf::from(name);
                candidate.is_file().then_some(candidate)
            })
    };
    if program.contains(['\\', '/', ':']) {
        return try_base(Path::new(program));
    }
    std::env::split_paths(path).find_map(|dir| try_base(&dir.join(program)))
}

/// On Windows, `program` resolved with [`find_program`] from the real
/// `PATH` and `PATHEXT`; elsewhere (or when not found) `program` itself.
fn resolve(program: &str) -> PathBuf {
    if cfg!(windows) {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
        if let Some(found) = find_program(program, &path, &pathext) {
            return found;
        }
    }
    PathBuf::from(program)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn finds_cmd_shims_with_pathext() {
        let dir = std::env::temp_dir().join(format!("baton-find-{}", std::process::id()));
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        // An npm-style install: a POSIX shim without extension and a .cmd.
        std::fs::write(bin.join("pi"), "#!/bin/sh").unwrap();
        std::fs::write(bin.join("pi.cmd"), "@echo off").unwrap();
        std::fs::write(bin.join("tool.EXE"), "").unwrap();
        let path = std::env::join_paths([dir.join("empty"), bin.clone()]).unwrap();
        let ext = ".COM;.EXE;.BAT;.CMD";
        // Windows may return `pi.CMD` for `pi.cmd`; compare without case.
        let low = |p: Option<PathBuf>| p.map(|p| p.to_string_lossy().to_lowercase());
        let find_program =
            |name: &str, path: &std::ffi::OsStr, ext: &str| low(find_program(name, path, ext));
        let want = |name: &str| low(Some(bin.join(name)));
        assert_eq!(find_program("pi", &path, ext), want("pi.cmd"));
        assert_eq!(find_program("pi.cmd", &path, ext), want("pi.cmd"));
        assert_eq!(find_program("tool", &path, ext), want("tool.EXE"));
        assert_eq!(find_program("missing", &path, ext), None);
        let full = bin.join("pi").to_string_lossy().into_owned();
        assert_eq!(find_program(&full, &path, ext), want("pi.cmd"));
        // Without PATHEXT, the extensionless shim is still never picked.
        assert_eq!(find_program("pi", &path, ""), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clean_command_on_this_os() {
        let cmd = clean_command(w(&["pi"]), None).unwrap();
        assert_eq!(cmd.last().unwrap(), "pi");
        if cfg!(unix) {
            assert_eq!(cmd[0], "env");
        } else {
            assert_eq!(cmd[1..3], ["exec", "--"]);
            let pi = clean_command(w(&["pi"]), Some(Agent::Pi)).unwrap();
            assert_eq!(
                pi[1..5],
                ["exec", "--default-env", "PI_PROGRAM_STATUS=1", "--"]
            );
        }
    }

    #[test]
    fn pi_gets_program_status_unless_set() {
        assert_eq!(default_env(Some(Agent::Pi)), ["PI_PROGRAM_STATUS=1"]);
        assert_eq!(default_env(Some(Agent::Claude)), [] as [&str; 0]);
        assert_eq!(default_env(None), [] as [&str; 0]);
        let pairs = w(&["PI_PROGRAM_STATUS=1", "A=x=y", "bad", "=v"]);
        assert_eq!(
            missing_env(&pairs, |_| false),
            [("PI_PROGRAM_STATUS", "1"), ("A", "x=y")]
        );
        // A value the user set is kept.
        assert_eq!(
            missing_env(&pairs, |k| k == "PI_PROGRAM_STATUS"),
            [("A", "x=y")]
        );
    }
}
