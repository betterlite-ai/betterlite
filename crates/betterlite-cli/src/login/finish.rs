use secrecy::{ExposeSecret as _, SecretString};

use crate::commands_listing;
use crate::credential_lifecycle::outcome::SecretSave;
use crate::credentials::notice::storage_notice;
use crate::session::configure::{ConfigureClaudeRequest, configure_claude_code};

use super::error::FlowError;
use super::ports::{FlowPorts, TerminalWriter};

pub const TOKEN_PREFIX_CHARS: usize = 20;
pub const SEPARATOR_WIDTH: usize = 60;

#[derive(Clone, Copy)]
pub struct LoginFinished<'finished> {
    pub base_url: &'finished str,
    pub api_key: &'finished SecretString,
    pub config_claude: bool,
    pub stored: &'finished SecretSave,
}

pub fn finish_login<H, C, E, W>(
    finished: LoginFinished<'_>,
    flow: &FlowPorts<'_, H, C, E, W>,
    stdout: &mut TerminalWriter,
) -> Result<(), FlowError> {
    writeln!(stdout, "\nLogin successful!")?;
    let prefix: String = finished
        .api_key
        .expose_secret()
        .chars()
        .take(TOKEN_PREFIX_CHARS)
        .collect();
    writeln!(stdout, "JWT Token: {prefix}...")?;
    writeln!(
        stdout,
        "{}",
        storage_notice(finished.stored, flow.token_path)
    )?;
    if finished.config_claude {
        configure_claude(finished.base_url, finished.api_key, flow, &mut *stdout)?;
    }
    if matches!(
        finished.stored,
        SecretSave::CredentialNotSaved(_) | SecretSave::CredentialNotRecorded(_)
    ) {
        if finished.config_claude {
            writeln!(
                stdout,
                "Claude Code was configured with this key even though the CLI itself could not keep it."
            )?;
        }
        return Ok(());
    }
    writeln!(
        stdout,
        "You can now use the CLI without specifying --api-key"
    )?;
    writeln!(stdout, "\n{}", "=".repeat(SEPARATOR_WIDTH))?;
    commands_listing::show_commands(&mut *stdout, flow.environment)?;
    Ok(())
}

fn configure_claude<H, C, E, W>(
    base_url: &str,
    api_key: &SecretString,
    flow: &FlowPorts<'_, H, C, E, W>,
    stdout: &mut TerminalWriter,
) -> Result<(), FlowError> {
    let mut request = ConfigureClaudeRequest {
        base_url,
        api_key,
        environment: flow.environment,
        files: flow.private_files,
        binary_path: flow.binary_path,
        stdout: &mut *stdout,
    };
    configure_claude_code(&mut request).map_err(FlowError::Configure)
}
