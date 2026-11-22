use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use tracing::debug;

use super::ports::TeamDirectory;

// a team's metadata.secret_manager_settings, kept as stored and never written to a log
pub type TeamSecretSettings = BTreeMap<String, JsonData>;

// _get_secret_manager_optional_params: only a lookup failure logs, every other gap is a quiet None
pub async fn secret_manager_settings(
    directory: Option<&dyn TeamDirectory>,
    requested_team: Option<&str>,
) -> Option<TeamSecretSettings> {
    let (Some(team_id), Some(teams)) = (requested_team, directory) else {
        return None;
    };
    let team = match teams.team(team_id).await {
        Ok(found) => found?,
        Err(error) => {
            debug!("Unable to load team metadata for team_id={team_id}: {error}");
            return None;
        }
    };
    let Some(JsonData::Object(settings)) = team
        .team
        .fields
        .base
        .metadata?
        .remove("secret_manager_settings")
    else {
        return None;
    };
    (!settings.is_empty()).then_some(settings)
}
