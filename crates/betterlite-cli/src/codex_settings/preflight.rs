use std::path::Path;

use super::document::read_settings;
use super::error::CodexSettingsError;
use super::receipt::read_receipt;
use super::version::{CodexVersion, codex_cli_version, require_safe_codex};

pub async fn preflight_codex_settings(
    settings_path: &Path,
    version: CodexVersion<'_>,
) -> Result<(), CodexSettingsError> {
    let reported = codex_cli_version(version).await;
    require_safe_codex(reported.as_deref())?;
    read_settings(settings_path).await?;
    read_receipt(settings_path).await?;
    Ok(())
}
