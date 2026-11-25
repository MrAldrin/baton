//! `baton` command line. See `README.md`.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Result, anyhow};
use clap::error::ErrorKind;
use clap::{Parser, Subcommand};

use baton::agent::Agent;
use baton::commands::{self, Ctx, Output, RunArgs};
use baton::pane::PaneId;
use baton::state::State;
use baton::store::Store;
use baton::zellij::Zellij;

/// Exit code for errors (bad arguments, missing pane, refused action).
const ERROR_EXIT: u8 = 3;

/// Run and drive programs in terminal multiplexer panes.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Session to act on. Default: $`ZELLIJ_SESSION_NAME`.
    #[arg(long, global = true)]
    session: Option<String>,
    #[command(subcommand)]
    verb: Verb,
}

#[derive(Debug, Subcommand)]
enum Verb {
    /// Start a program in a new pane without focus, and record ownership.
    Run {
        /// Pane name.
        #[arg(long)]
        name: String,
        /// Working folder.
        #[arg(long)]
        cwd: PathBuf,
        /// Tab for the floating pane. Default: the caller's own tab.
        #[arg(long, conflicts_with = "new_tab")]
        tab_id: Option<u64>,
        /// Open a new tab with this name, and type the command into its shell.
        #[arg(long)]
        new_tab: Option<String>,
        /// Agent: sets a session id so the pane can be resumed later.
        #[arg(long, value_enum)]
        agent: Option<Agent>,
        /// The command (after --). Optional with --agent.
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Type text and submit it.
    Send {
        pane: PaneId,
        text: String,
        /// Do not press Enter.
        #[arg(long)]
        no_enter: bool,
        /// Send to a busy pane, or to a pane baton did not start.
        #[arg(long)]
        force: bool,
    },
    /// Send keys: enter, esc, tab, up, down, ctrl-c, or digits.
    Keys {
        pane: PaneId,
        #[arg(required = true)]
        keys: Vec<String>,
        /// Send to a pane baton did not start.
        #[arg(long)]
        force: bool,
    },
    /// Print the pane's text.
    Read {
        pane: PaneId,
        /// Include the scrollback.
        #[arg(long)]
        full: bool,
        /// Only the last N lines.
        #[arg(long)]
        lines: Option<usize>,
        /// Print JSON instead of plain text.
        #[arg(long)]
        json: bool,
    },
    /// Print the pane's state.
    Status { pane: PaneId },
    /// Block until the pane reaches a state after the last send.
    Wait {
        pane: PaneId,
        /// States to stop on.
        #[arg(
            long,
            value_enum,
            value_delimiter = ',',
            default_value = "done,blocked,error,exited"
        )]
        until: Vec<State>,
        /// Give up after this many seconds (exit 124).
        #[arg(long)]
        timeout: Option<f64>,
    },
    /// List terminal panes.
    List {
        /// Only panes baton started.
        #[arg(long)]
        owned: bool,
    },
    /// Internal: called by agent hooks to report state (OSC 7501).
    #[command(hide = true)]
    Hook {
        #[arg(value_enum)]
        agent: Agent,
    },
    /// Close a pane baton started, and remove its record.
    Close {
        pane: PaneId,
        /// Close a pane baton did not start.
        #[arg(long)]
        force: bool,
    },
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) => {
            let _ = e.print();
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            print_error(&e.to_string());
            return ExitCode::from(ERROR_EXIT);
        }
    };
    if let Verb::Hook { agent } = cli.verb {
        hook(agent);
        return ExitCode::SUCCESS;
    }
    match execute(cli) {
        Ok(out) => {
            let mut stdout = std::io::stdout().lock();
            let _ = match out.output {
                Output::Json(v) => writeln!(stdout, "{v}"),
                Output::Text(t) => write!(stdout, "{t}"),
            };
            ExitCode::from(u8::try_from(out.code).unwrap_or(ERROR_EXIT))
        }
        Err(e) => {
            print_error(&format!("{e:#}"));
            ExitCode::from(ERROR_EXIT)
        }
    }
}

/// Report an agent hook event as OSC 7501. Never fails and prints nothing:
/// a failing or talking hook can block the agent.
fn hook(agent: Agent) {
    use std::io::Read;
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let Ok(event) = serde_json::from_str::<serde_json::Value>(&input) else {
        return;
    };
    let report = match agent {
        Agent::Claude => baton::hook::claude_report(&event),
        Agent::Pi => None,
    };
    if let Some(r) = report {
        let _ =
            baton::hook::write_to_agent_terminal(baton::hook::osc(&r, agent.program()).as_bytes());
    }
}

fn print_error(message: &str) {
    let v = serde_json::json!({ "error": { "message": message.trim() } });
    println!("{v}");
}

fn execute(cli: Cli) -> Result<commands::Outcome> {
    let env_session = std::env::var("ZELLIJ_SESSION_NAME")
        .ok()
        .filter(|s| !s.is_empty());
    let session = cli
        .session
        .or_else(|| env_session.clone())
        .ok_or_else(|| anyhow!("no session: give --session or run inside Zellij"))?;
    // $ZELLIJ_PANE_ID names a pane of $ZELLIJ_SESSION_NAME only.
    let caller_pane = if env_session.as_deref() == Some(session.as_str()) {
        std::env::var("ZELLIJ_PANE_ID")
            .ok()
            .and_then(|p| p.parse().ok())
    } else {
        None
    };
    let host = Zellij::connect(&session)?;
    let store = Store::default_location()?;
    let ctx = Ctx {
        host: &host,
        store: &store,
        caller_pane,
    };
    match cli.verb {
        Verb::Run {
            name,
            cwd,
            tab_id,
            new_tab,
            agent,
            command,
        } => commands::run(
            &ctx,
            RunArgs {
                name,
                cwd,
                tab_id,
                new_tab,
                agent,
                command,
                baton_exe: std::env::current_exe()?,
            },
        ),
        Verb::Send {
            pane,
            text,
            no_enter,
            force,
        } => commands::send(&ctx, pane, &text, !no_enter, force),
        Verb::Keys { pane, keys, force } => commands::keys(&ctx, pane, &keys, force),
        Verb::Read {
            pane,
            full,
            lines,
            json,
        } => commands::read(&ctx, pane, full, lines, json),
        Verb::Status { pane } => commands::status(&ctx, pane),
        Verb::Wait {
            pane,
            until,
            timeout,
        } => {
            let timeout = timeout
                .map(Duration::try_from_secs_f64)
                .transpose()
                .map_err(|e| anyhow!("bad --timeout: {e}"))?;
            commands::wait(&ctx, pane, &until, timeout, Duration::from_millis(500))
        }
        Verb::List { owned } => commands::list(&ctx, owned),
        Verb::Close { pane, force } => commands::close(&ctx, pane, force),
        Verb::Hook { .. } => unreachable!("handled in main"),
    }
}
