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
//!   returns its exit code ([`exec`]).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, ensure};

use crate::commands::{CLAUDE_HOST_VARS, shell_join, without_host_vars};

/// The command that a pane runs for `command`: the OS launcher in front of
/// it, so the program starts without [`CLAUDE_HOST_VARS`].
///
/// # Errors
/// On Windows, the path of the running `baton` is not known.
pub fn clean_command(command: Vec<String>) -> Result<Vec<String>> {
    if cfg!(unix) {
        Ok(without_host_vars(command))
    } else {
        let exe = std::env::current_exe().context("cannot find the baton program")?;
        let mut out = vec![
            plain_path(&exe).to_string_lossy().into_owned(),
            "exec".to_owned(),
            "--".to_owned(),
        ];
        out.extend(command);
        Ok(out)
    }
}

/// `baton exec`: start `command` without [`CLAUDE_HOST_VARS`], wait for it
/// and return its exit code. On Unix the program replaces `baton`.
///
/// # Errors
/// The command is empty or cannot start.
pub fn exec(command: &[String]) -> Result<i32> {
    let (program, args) = command.split_first().context("give a command after --")?;
    let mut cmd = Command::new(program);
    cmd.args(args);
    for var in CLAUDE_HOST_VARS {
        cmd.env_remove(var);
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

/// The shell in a new tab, which decides how `run --new-tab` quotes the
/// command it types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    /// sh, bash, zsh, fish (and any unknown shell).
    Posix,
    /// `PowerShell` 5 (`powershell.exe`) or 7 (`pwsh`).
    PowerShell,
    /// `cmd.exe`.
    Cmd,
}

impl Shell {
    /// The shell for a pane command such as `C:\…\pwsh.exe -NoLogo`. On
    /// Unix this is always [`Shell::Posix`].
    #[must_use]
    pub fn detect(command: Option<&str>) -> Self {
        if cfg!(unix) {
            return Self::Posix;
        }
        Self::from_command(command.unwrap_or_default())
    }

    /// The shell named by the program of `command`, on any OS.
    ///
    /// ```
    /// use baton::launch::Shell;
    /// assert_eq!(Shell::from_command(r"C:\Program Files\PowerShell\7\pwsh.exe"), Shell::PowerShell);
    /// assert_eq!(Shell::from_command("cmd.exe /k"), Shell::Cmd);
    /// assert_eq!(Shell::from_command("/bin/bash"), Shell::Posix);
    /// ```
    #[must_use]
    pub fn from_command(command: &str) -> Self {
        let lower = command.to_ascii_lowercase();
        let program = lower
            .split(".exe")
            .next()
            .unwrap_or_default()
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .next()
            .unwrap_or_default();
        match program {
            "pwsh" | "powershell" => Self::PowerShell,
            "cmd" => Self::Cmd,
            _ => Self::Posix,
        }
    }

    /// Join words into one command line for this shell.
    ///
    /// # Errors
    /// cmd.exe: a word has `"`, `%` or a line break, which cmd.exe cannot
    /// pass on safely.
    pub fn join(self, words: &[String]) -> Result<String> {
        match self {
            Self::Posix => Ok(shell_join(words)),
            Self::PowerShell => Ok(powershell_join(words)),
            Self::Cmd => cmd_join(words),
        }
    }
}

fn plain(word: &str) -> bool {
    !word.is_empty()
        && word
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_./=:,+@\\".contains(&b))
}

/// `& 'prog' 'arg' …`: single quotes are literal in `PowerShell`; a `'` is
/// doubled. The call operator `&` runs a quoted program path. Only words of
/// letters, digits and `_ . / \ :` stay bare: `PowerShell` gives `,` `@`
/// `$` and a leading `-` (`--` too) their own meaning.
fn powershell_join(words: &[String]) -> String {
    let mut out = String::from("&");
    for w in words {
        out.push(' ');
        let bare = !w.is_empty()
            && w.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_./\\:".contains(&b));
        if bare {
            out.push_str(w);
        } else {
            out.push('\'');
            out.push_str(&w.replace('\'', "''"));
            out.push('\'');
        }
    }
    out
}

/// Words in double quotes where needed. Inside `"…"` cmd.exe keeps
/// `& | < > ^` literal, but it still expands `%VAR%`, and a `"` ends the
/// quote, so words with `"` or `%` are refused.
fn cmd_join(words: &[String]) -> Result<String> {
    let mut parts = Vec::with_capacity(words.len());
    for w in words {
        ensure!(
            !w.contains(['"', '%', '\n', '\r']),
            "cmd.exe cannot pass {w:?} safely; use a tab with PowerShell, or a floating pane"
        );
        if plain(w) {
            parts.push(w.clone());
        } else {
            parts.push(format!("\"{w}\""));
        }
    }
    Ok(parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn powershell_quotes() {
        assert_eq!(
            Shell::PowerShell
                .join(&w(&[
                    r"C:\Program Files\baton.exe",
                    "exec",
                    "--",
                    "it's",
                    "a"
                ]))
                .unwrap(),
            r"& 'C:\Program Files\baton.exe' exec '--' 'it''s' a"
        );
        // A plain program path needs no quotes, but `&` still runs it.
        assert_eq!(
            Shell::PowerShell.join(&w(&["pi", "$x", ""])).unwrap(),
            "& pi '$x' ''"
        );
    }

    #[test]
    fn cmd_quotes_and_refuses() {
        assert_eq!(
            Shell::Cmd
                .join(&w(&[r"C:\Program Files\baton.exe", "exec", "a&b", ""]))
                .unwrap(),
            r#""C:\Program Files\baton.exe" exec "a&b" """#
        );
        assert!(Shell::Cmd.join(&w(&["echo", "%PATH%"])).is_err());
        assert!(Shell::Cmd.join(&w(&["echo", "a\"b"])).is_err());
    }

    #[test]
    fn posix_is_shell_join() {
        let words = w(&["pi", "it's ok"]);
        assert_eq!(Shell::Posix.join(&words).unwrap(), shell_join(&words));
    }

    #[test]
    fn detect_shells() {
        assert_eq!(Shell::from_command("powershell.exe"), Shell::PowerShell);
        assert_eq!(Shell::from_command("PWSH -NoLogo"), Shell::PowerShell);
        assert_eq!(
            Shell::from_command(r"C:\Windows\System32\cmd.exe"),
            Shell::Cmd
        );
        assert_eq!(Shell::from_command("nu"), Shell::Posix);
        if cfg!(unix) {
            assert_eq!(Shell::detect(Some("pwsh")), Shell::Posix);
        }
    }

    #[test]
    fn clean_command_on_this_os() {
        let cmd = clean_command(w(&["pi"])).unwrap();
        assert_eq!(cmd.last().unwrap(), "pi");
        if cfg!(unix) {
            assert_eq!(cmd[0], "env");
        } else {
            assert_eq!(cmd[1..3], ["exec", "--"]);
        }
    }
}
