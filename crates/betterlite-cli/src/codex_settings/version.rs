use std::path::Path;
use std::time::Duration;

use regex::Regex;
use tokio::process as tokio_process;
use tokio::time;

use super::error::CodexSettingsError;

pub const MIN_CODEX_VERSION: (u64, u64, u64) = (0, 129, 0);
pub const CODEX_VERSION_TIMEOUT: Duration = Duration::from_secs(5);

const CODEX_VERSION_PATTERN: &str = r"^codex-cli ([0-9]+)\.([0-9]+)\.([0-9]+)$";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodexVersion<'version> {
    Installed,
    Reported(Option<&'version str>),
}

pub async fn codex_cli_version(source: CodexVersion<'_>) -> Option<String> {
    match source {
        CodexVersion::Installed => {
            installed_codex_version(Path::new("codex"), CODEX_VERSION_TIMEOUT).await
        }
        CodexVersion::Reported(reported) => reported.map(str::to_owned),
    }
}

// subprocess.run kills the probe it started after its timeout; kill_on_drop does the same for the tokio child
pub async fn installed_codex_version(program: &Path, limit: Duration) -> Option<String> {
    let probe = tokio_process::Command::new(program)
        .arg("--version")
        .kill_on_drop(true)
        .output();
    let output = time::timeout(limit, probe).await.ok()?.ok()?;
    let stdout = String::from_utf8(output.stdout).ok()?;
    output.status.success().then_some(stdout)
}

pub fn require_safe_codex(version: Option<&str>) -> Result<(), CodexSettingsError> {
    match version.and_then(parse_codex_version) {
        Some(found) if found >= MIN_CODEX_VERSION => Ok(()),
        Some(_) | None => Err(CodexSettingsError::UnsafeCodex),
    }
}

fn parse_codex_version(output: &str) -> Option<(u64, u64, u64)> {
    let pattern = Regex::new(CODEX_VERSION_PATTERN).ok()?;
    let captures = pattern.captures(output.trim())?;
    Some((
        captures.get(1)?.as_str().parse().ok()?,
        captures.get(2)?.as_str().parse().ok()?,
        captures.get(3)?.as_str().parse().ok()?,
    ))
}

mod tests;
