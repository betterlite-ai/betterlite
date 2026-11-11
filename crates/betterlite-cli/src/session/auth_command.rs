use clap::Subcommand;

use crate::pkce_login::{Http, WallClock};

use super::error::SessionError;
use super::print_token::{PrintTokenRequest, print_token};

pub const AUTH_ABOUT: &str = "Manage CLI authentication (apiKeyHelper support, etc.)";

#[derive(Clone, Copy, Debug, Subcommand)]
pub enum AuthCommand {
    #[command(
        name = "print-token",
        about = "Print a valid API token for this proxy.",
        long_about = "Print a valid API token for this proxy.\n\nDesigned to be used as Claude Code's `apiKeyHelper` (https://docs.claude.com/en/docs/claude-code/settings): stdout must contain only the token, so all diagnostics go to stderr. The token expires after `LITELLM_CLI_JWT_EXPIRATION_HOURS` (default 24h); a `lite login --pkce` token renews itself here first, and once a token has expired for good, run the same `lite login` command again."
    )]
    PrintToken,
}

pub fn run_auth_command<H, W>(
    command: AuthCommand,
    request: &mut PrintTokenRequest<'_, H, W>,
) -> Result<(), SessionError>
where
    H: Http + Send + Sync + 'static,
    W: WallClock + Send + Sync + 'static,
{
    match command {
        AuthCommand::PrintToken => print_token(request),
    }
}
