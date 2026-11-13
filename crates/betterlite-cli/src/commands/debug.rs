pub mod error;
pub mod fetch;
pub mod render;
pub mod report_file;
pub mod session;
pub mod slash_command;
pub mod wire;

use std::io::Write;
use std::path::Path;

use betterlite_config::loader::EnvironmentLookup;
use clap::{Args, Subcommand};

use self::error::DebugError;
use self::fetch::SpendLogsFetcher;
use self::render::{Clock, ReportContext, build_report};
use self::report_file::write_report;
use self::session::detect_claude_session_id;
use self::slash_command::{SLASH_COMMAND_NAME, install_slash_command};
use super::Client;
use crate::cli_context::CliContext;

const DEFAULT_RECENT_BODIES: u32 = 3;
const DEFAULT_MAX_BODY_CHARS: u32 = 20_000;
const MIN_BODY_CHARS: i64 = 100;

#[derive(Debug, Subcommand)]
#[command(about = "Pull debug reports (spend, request, response, error) for coding-agent sessions")]
pub enum DebugCommand {
    #[command(
        about = "Render a markdown debug report for one Claude Code session routed through the proxy"
    )]
    Claude(ClaudeOptions),
    #[command(
        about = "Install the /debug-lite slash command into ~/.claude/commands so Claude Code can run `lite debug claude`"
    )]
    InstallClaudeCommand,
}

#[derive(Debug, Args)]
pub struct ClaudeOptions {
    #[arg(
        long,
        help = "Claude Code session id. Defaults to $CLAUDE_CODE_SESSION_ID, else the most recently used transcript in ~/.claude"
    )]
    session_id: Option<String>,
    #[arg(
        long,
        default_value_t = DEFAULT_RECENT_BODIES,
        value_parser = clap::value_parser!(u32),
        help = "Also include request/response bodies for the N most recent turns (failed turns always get bodies)"
    )]
    recent_bodies: u32,
    #[arg(
        long,
        default_value_t = DEFAULT_MAX_BODY_CHARS,
        value_parser = clap::value_parser!(u32).range(MIN_BODY_CHARS..),
        help = "Truncate each request/response body to this many characters"
    )]
    max_body_chars: u32,
    #[arg(
        long,
        help = "Print only, do not write the report under ~/.litellm/debug"
    )]
    no_save: bool,
}

pub struct DebugPorts<'port> {
    pub context: &'port CliContext,
    pub client: &'port Client,
    pub environment: &'port dyn EnvironmentLookup,
    pub clock: &'port dyn Clock,
    pub stdout: &'port mut dyn Write,
    pub stderr: &'port mut dyn Write,
}

pub fn run(command: DebugCommand, ports: DebugPorts<'_>) -> Result<(), DebugError> {
    let home = ports.environment.system_home().ok_or(DebugError::NoHome)?;
    match command {
        DebugCommand::Claude(options) => run_claude(&options, ports, &home),
        DebugCommand::InstallClaudeCommand => {
            let path = install_slash_command(&home.join(".claude"))?;
            writeln!(
                ports.stdout,
                "Installed /{SLASH_COMMAND_NAME}: {}",
                path.display()
            )?;
            writeln!(
                ports.stdout,
                "Restart Claude Code (or start a new session), then type /{SLASH_COMMAND_NAME}."
            )?;
            Ok(())
        }
    }
}

fn run_claude(
    options: &ClaudeOptions,
    ports: DebugPorts<'_>,
    home: &Path,
) -> Result<(), DebugError> {
    let session_id = match options.session_id.as_deref() {
        Some(explicit) if !explicit.is_empty() => explicit.to_owned(),
        Some(_) | None => detect_claude_session_id(ports.environment, &home.join(".claude"))?
            .ok_or(DebugError::NoSession)?,
    };
    let fetcher = SpendLogsFetcher {
        client: ports.client,
    };
    let rows = fetcher.session_rows(&session_id)?;
    let payloads = fetcher.payloads(
        &rows,
        usize::try_from(options.recent_bodies).unwrap_or(usize::MAX),
    )?;
    let context = ReportContext {
        session_id: &session_id,
        base_url: &ports.context.base_url,
        max_body_chars: usize::try_from(options.max_body_chars).unwrap_or(usize::MAX),
        clock: ports.clock,
    };
    let report = build_report(&context, &rows, &payloads)?;
    writeln!(ports.stdout, "{report}")?;
    if !options.no_save {
        let path = write_report(&report, &session_id, &home.join(".litellm").join("debug"))?;
        writeln!(ports.stderr, "Saved to {}", path.display())?;
    }
    Ok(())
}

mod tests;
