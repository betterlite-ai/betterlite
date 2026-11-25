use std::collections::BTreeSet;

use betterlite_models::model_registration::JsonObject;

use crate::model_info_scope::text_field;
use crate::spend::visibility::roles::is_admin_view_safe;
use crate::team_access::caller::TeamCaller;

use super::reads::TeamScopeReads;

pub async fn get_caller_byok_team_scope<Reads>(
    caller: Option<&TeamCaller>,
    reads: Option<&Reads>,
) -> Option<BTreeSet<String>>
where
    Reads: TeamScopeReads + ?Sized,
{
    let (Some(requester), Some(store)) = (caller, reads) else {
        return None;
    };
    if is_admin_view_safe(requester) {
        return None;
    }
    let key_team_scope: BTreeSet<String> = requester
        .team_id
        .iter()
        .filter(|team| !team.is_empty())
        .cloned()
        .collect();
    let Some(user_id) = requester.user_id.as_deref() else {
        return Some(key_team_scope);
    };
    let listed = match store.user_teams(user_id).await {
        Ok(listed) => listed,
        Err(error) => {
            tracing::error!(
                "Failed to look up caller teams while scoping BYOK search; defaulting to key team scope only. {error}"
            );
            return Some(key_team_scope);
        }
    };
    let mut scope = key_team_scope;
    if let Some(members) = listed {
        scope.extend(members);
    }
    Some(scope)
}

#[must_use]
pub fn byok_row_outside_caller_teams(
    model_info: &JsonObject,
    allowed_team_ids: Option<&BTreeSet<String>>,
) -> bool {
    let Some(allowed) = allowed_team_ids else {
        return false;
    };
    let Some(team_id) = text_field(model_info, "team_id") else {
        return false;
    };
    !allowed.contains(team_id)
}
