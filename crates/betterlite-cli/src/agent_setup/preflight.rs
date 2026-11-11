use betterlite_config::loader::EnvironmentLookup;

use super::error::AgentSetupError;
use crate::agent_launch::codex_config_path;
use crate::agent_setup_profiles::Target;
use crate::agent_setup_profiles::error::AgentSetupProfilesError;
use crate::claude_settings::paths::{claude_settings_path, claude_settings_paths};
use crate::claude_settings::receipts::preflight::preflight_claude_settings;
use crate::codex_settings::preflight::preflight_codex_settings;
use crate::codex_settings::version::CodexVersion;

pub async fn preflight(
    target: Target,
    environment: &(dyn EnvironmentLookup + Send + Sync),
    version: CodexVersion<'_>,
) -> Result<(), AgentSetupError> {
    match target {
        Target::Claude => {
            let home = environment.system_home().ok_or_else(|| {
                AgentSetupError::Profile(Box::new(AgentSetupProfilesError::Path(Target::Claude)))
            })?;
            let settings_path = claude_settings_path(environment).ok_or_else(|| {
                AgentSetupError::Profile(Box::new(AgentSetupProfilesError::Path(Target::Claude)))
            })?;
            preflight_claude_settings(&settings_path, &claude_settings_paths(&home))?;
            Ok(())
        }
        Target::Codex => {
            let settings_path = codex_config_path(environment).ok_or_else(|| {
                AgentSetupError::Profile(Box::new(AgentSetupProfilesError::Path(Target::Codex)))
            })?;
            preflight_codex_settings(&settings_path, version).await?;
            Ok(())
        }
    }
}
