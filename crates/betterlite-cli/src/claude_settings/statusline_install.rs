use std::borrow::Cow;
use std::env;
use std::mem;
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

// A user's status line can end in the same subcommand word, so the binary name has to match too
#[must_use]
pub fn is_our_statusline(existing: &str, command: &str) -> bool {
    let posix_ours = shlex::split(command).unwrap_or_default();
    let posix_theirs = shlex::split(existing).unwrap_or_default();
    let cmd_ours = cmd_words(command);
    let cmd_theirs = cmd_words(existing);
    same_status_command(&posix_ours, &posix_theirs) || same_status_command(&cmd_ours, &cmd_theirs)
}

fn same_status_command(ours: &[String], theirs: &[String]) -> bool {
    let [our_binary, our_subcommand] = ours else {
        return false;
    };
    let [their_binary, their_subcommand] = theirs else {
        return false;
    };
    our_subcommand == STATUSLINE_SUBCOMMAND
        && their_subcommand == our_subcommand
        && our_binary.rsplit(['/', '\\']).next() == their_binary.rsplit(['/', '\\']).next()
}

// quote_for_cmd wraps every token in double quotes and doubles a quote inside one
fn cmd_words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut characters = command.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' if quoted && characters.peek() == Some(&'"') => {
                characters.next();
                current.push('"');
            }
            '"' => quoted = !quoted,
            ' ' | '\t' if !quoted => {
                if !current.is_empty() {
                    words.push(mem::take(&mut current));
                }
            }
            other => current.push(other),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

mod tests;
