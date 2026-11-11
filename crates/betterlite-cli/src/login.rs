pub mod echo;
pub mod error;
pub mod finish;
pub mod pkce;
pub mod ports;
pub mod preflight;
pub mod sso;
pub mod store;

use clap::Args;

use crate::pkce_login::{Clock, Entropy, Http, WallClock};
use crate::sso_polling::transport::PollTransport;

use error::{FlowError, LoginError};
use ports::{FlowPorts, LoginPorts, TerminalWriter};

pub const LOGIN_ABOUT: &str = "Login to LiteLLM proxy using SSO authentication";
pub const CONFIG_CLAUDE_HELP: &str = "After logging in, write this login's key into ~/.claude/settings.json so Claude Code routes through this proxy; run it again after the key expires. Unrelated settings are preserved.";
pub const PKCE_HELP: &str = "Sign in with OAuth authorization code + PKCE through your system browser (loopback redirect), with a refresh token that renews the key automatically. Requires a proxy that serves /.well-known/litellm-cli-auth.";

#[derive(Args, Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LoginArgs {
    #[arg(long, help = CONFIG_CLAUDE_HELP)]
    pub config_claude: bool,
    #[arg(long, help = PKCE_HELP)]
    pub pkce: bool,
}

pub trait Interrupts {
    fn interrupted(&self) -> bool;
}

pub struct NeverInterrupted;

impl Interrupts for NeverInterrupted {
    fn interrupted(&self) -> bool {
        false
    }
}

pub fn run_login<H, C, E, W>(
    args: LoginArgs,
    ports: &mut LoginPorts<'_, H, C, E, W>,
) -> Result<(), LoginError>
where
    H: Http + PollTransport + Send + Sync + 'static,
    C: Clock,
    E: Entropy,
    W: WallClock + Send + Sync + 'static,
{
    if args.config_claude {
        preflight::refuse_to_start(ports.flow.environment)
            .map_err(|source| LoginError::NotLoggingIn { source })?;
    }
    let flow = if args.pkce {
        pkce::pkce_login(args.config_claude, &mut ports.flow, &mut *ports.stdout)
    } else {
        sso::sso_login(args.config_claude, &mut ports.flow, &mut *ports.stdout)
    };
    match flow {
        Ok(()) => Ok(()),
        Err(FlowError::Configure(source)) => Err(LoginError::ConfigureClaude { source }),
        Err(failure) => report_failure(&failure, &ports.flow, &mut *ports.stdout),
    }
}

fn report_failure<H, C, E, W>(
    failure: &FlowError,
    flow: &FlowPorts<'_, H, C, E, W>,
    stdout: &mut TerminalWriter,
) -> Result<(), LoginError> {
    if flow.interrupt.interrupted() {
        writeln!(stdout, "\nAuthentication cancelled by user.")
            .map_err(|source| LoginError::Write { source })?;
        return Ok(());
    }
    writeln!(stdout, "Authentication failed: {failure}")
        .map_err(|source| LoginError::Write { source })
}

mod tests;
