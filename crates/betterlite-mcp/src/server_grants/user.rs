use betterlite_auth::ui_session_identity::UI_SESSION_TOKEN_TEAM_ID;
use indexmap::IndexSet;

// _resolve_user_team_ids keeps the caller's teams distinct, without the empty id or the dashboard
pub(super) fn distinct_team_ids(teams: Vec<String>) -> Vec<String> {
    let distinct: IndexSet<String> = teams
        .into_iter()
        .filter(|team_id| !team_id.is_empty() && team_id.as_str() != UI_SESSION_TOKEN_TEAM_ID)
        .collect();
    distinct.into_iter().collect()
}
