use std::borrow::Cow;
use std::env;
use std::path::Path;

use super::error::ClaudeSettingsError;
use crate::cmd_quoting::quote_for_cmd;

const STATUSLINE_SUBCOMMAND: &str = "statusline";

pub fn install_statusline_script(
    binary_path: Option<&Path>,
) -> Result<String, ClaudeSettingsError> {
    let binary = match binary_path {
        Some(path) => path.to_owned(),
        None => env::current_exe().map_err(|source| ClaudeSettingsError::CurrentExe { source })?,
    };
    statusline_command(&binary, env::consts::OS)
}

pub fn statusline_command(
    binary_path: &Path,
    platform: &str,
) -> Result<String, ClaudeSettingsError> {
    let binary = binary_path
        .to_str()
        .ok_or_else(|| ClaudeSettingsError::NonUtf8Path {
            path: binary_path.to_owned(),
        })?;
    if platform.starts_with("win") {
        return Ok(format!(
            "{} {}",
            quote_for_cmd(binary),
            quote_for_cmd(STATUSLINE_SUBCOMMAND)
        ));
    }
    let quoted_binary = quote_in_shell(binary, binary_path)?;
    let quoted_subcommand = quote_in_shell(STATUSLINE_SUBCOMMAND, binary_path)?;
    Ok(format!("{quoted_binary} {quoted_subcommand}"))
}

fn quote_in_shell(token: &str, path: &Path) -> Result<String, ClaudeSettingsError> {
    shlex::try_quote(token)
        .map(Cow::into_owned)
        .map_err(|source| ClaudeSettingsError::Quote {
            path: path.to_owned(),
            source,
        })
}

mod tests;
