use betterlite_db::budget_permission_rows::JsonData;

use super::error::KeyListFiltersError;
use super::ports::KeyListFiltersStore;
use crate::team_access::caller::TeamCaller;

// _apply_non_admin_alias_scope: aliases of the caller's keys or of the teams in their user row
pub async fn apply_non_admin_alias_scope(
    caller: &TeamCaller,
    store: &dyn KeyListFiltersStore,
    query_params: &mut Vec<JsonData>,
    where_parts: &mut Vec<String>,
) -> Result<(), KeyListFiltersError> {
    let identity = caller.user_id.as_deref().filter(|id| !id.is_empty());
    let mut conditions: Vec<String> = Vec::new();
    if let Some(caller_id) = identity {
        query_params.push(JsonData::String(caller_id.to_owned()));
        conditions.push(format!("user_id = ${}", query_params.len()));
    }
    let teams = match identity {
        Some(caller_id) => store
            .user_row(caller_id)
            .await?
            .map(|row| row.teams)
            .unwrap_or_default(),
        None => Vec::new(),
    };
    if !teams.is_empty() {
        let offset = query_params.len();
        let placeholders: Vec<String> = (1..=teams.len())
            .map(|position| format!("${}", offset + position))
            .collect();
        query_params.extend(teams.into_iter().map(JsonData::String));
        conditions.push(format!("team_id IN ({})", placeholders.join(", ")));
    }
    if conditions.is_empty() {
        where_parts.push("FALSE".to_owned());
    } else {
        where_parts.push(format!("({})", conditions.join(" OR ")));
    }
    Ok(())
}
