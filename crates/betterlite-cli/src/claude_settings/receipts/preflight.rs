use std::path::Path;

use super::super::io::{env_object, load_json_or_empty};
use super::super::paths::{
    ClaudeSettingsPaths, configure_state_path, refuse_while_owned, settings_file_owners,
};
use super::error::ReceiptsError;
use super::read::read_configure_receipt;

pub fn preflight_claude_settings(
    settings_path: &Path,
    paths: &ClaudeSettingsPaths,
) -> Result<(), ReceiptsError> {
    refuse_while_owned(settings_path, &settings_file_owners(settings_path, paths)?)?;
    let settings = load_json_or_empty(settings_path)?;
    env_object(&settings, settings_path)?;
    let _unused = read_configure_receipt(&configure_state_path(settings_path, paths)?)?;
    Ok(())
}
