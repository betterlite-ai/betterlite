use std::fs;
use std::path::{Path, PathBuf};

use super::error::DebugError;

pub const SLASH_COMMAND_NAME: &str = "debug-lite";
pub const SLASH_COMMAND_BODY: &str = "---
description: Pull the LiteLLM debug report (spend, request, response, error) for this Claude Code session
allowed-tools: Bash(lite debug claude:*)
---
Below is the LiteLLM debug report for this Claude Code session. Summarize the failing
request(s) in a few sentences (model, error, request id) and tell me the path the full
report was saved to so I can hand it off. If nothing failed, say so.

!`lite debug claude $ARGUMENTS`
";

pub fn install_slash_command(claude_dir: &Path) -> Result<PathBuf, DebugError> {
    let commands_dir = claude_dir.join("commands");
    fs::create_dir_all(&commands_dir).map_err(|source| DebugError::SlashCommandDirectory {
        path: commands_dir.clone(),
        source,
    })?;
    let path = commands_dir.join(format!("{SLASH_COMMAND_NAME}.md"));
    fs::write(&path, SLASH_COMMAND_BODY).map_err(|source| DebugError::SlashCommandWrite {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

mod tests;
