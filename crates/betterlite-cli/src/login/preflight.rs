use betterlite_config::loader::EnvironmentLookup;

use crate::claude_settings::paths::{
    claude_settings_path, claude_settings_paths, refuse_while_owned, settings_file_owners,
};

use super::error::PreflightError;

pub fn refuse_to_start(environment: &dyn EnvironmentLookup) -> Result<(), PreflightError> {
    let settings = claude_settings_path(environment).ok_or(PreflightError::NoHome)?;
    let home = environment.system_home().ok_or(PreflightError::NoHome)?;
    let paths = claude_settings_paths(&home);
    let owners = settings_file_owners(&settings, &paths)?;
    refuse_while_owned(&settings, &owners)?;
    Ok(())
}
