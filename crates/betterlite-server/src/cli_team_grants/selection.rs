use betterlite_config::sections::settings::JsonValue;

use super::records::{CliSsoTeamDetail, TEAMLESS_CLI_SSO_TEAM_DETAIL};

mod tests;

// None means the team's grants are unknown, which a caller must not mint as an empty grant
#[must_use]
pub fn selected_cli_sso_team_detail(
    details: &[CliSsoTeamDetail],
    team_id: Option<&str>,
) -> Option<CliSsoTeamDetail> {
    let Some(wanted_team_id) = team_id else {
        return Some(TEAMLESS_CLI_SSO_TEAM_DETAIL);
    };
    details
        .iter()
        .find(|detail| detail.team_id.as_deref() == Some(wanted_team_id))
        .cloned()
}

#[must_use]
// ast-grep-ignore: no-long-names - LiteLLM's selected_cli_sso_team_detail over the cached details value
pub fn selected_cached_cli_sso_team_detail(
    cached: Option<&JsonValue>,
    team_id: Option<&str>,
) -> Option<CliSsoTeamDetail> {
    if team_id.is_none() {
        return Some(TEAMLESS_CLI_SSO_TEAM_DETAIL);
    }
    let details: Vec<CliSsoTeamDetail> = serde_json::from_value(cached?.clone()).ok()?;
    selected_cli_sso_team_detail(&details, team_id)
}
