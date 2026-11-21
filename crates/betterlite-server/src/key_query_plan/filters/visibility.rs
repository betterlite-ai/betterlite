use crate::key_query_plan::filters::{
    KeyColumn, KeyFilters, KeyScalar, Where, WhereClause, non_empty,
};

// every branch LiteLLM ORs together before the global filters narrow them again
#[must_use]
pub fn visibility_arms(filters: &KeyFilters) -> Vec<WhereClause> {
    let mut arms = Vec::new();
    let own = own_arm(filters);
    if !own.is_empty() {
        arms.push(own);
    }
    if filters.include_created_by_keys
        && let Some(user_id) = non_empty(&filters.user_id)
    {
        arms.push(created_by_arm(user_id, filters.member_team_ids.as_deref()));
    }
    let admin_teams = filters
        .admin_team_ids
        .as_deref()
        .filter(|ids| !ids.is_empty());
    if let Some(admin_team_ids) = admin_teams {
        arms.push(vec![Where::In(KeyColumn::TeamId, admin_team_ids.to_vec())]);
    }
    if let Some(arm) = member_service_account_arm(filters) {
        arms.push(arm);
    }
    arms
}

// the caller's own keys; exclude_team_id and organization_id belong to this arm alone
fn own_arm(filters: &KeyFilters) -> WhereClause {
    let mut entries = Vec::new();
    if let Some(user_id) = non_empty(&filters.user_id) {
        entries.push(Where::matching(
            KeyColumn::UserId,
            filters.user_matching,
            user_id,
        ));
    }
    if let Some(team_id) = non_empty(&filters.exclude_team_id) {
        entries.push(Where::NotEquals(
            KeyColumn::TeamId,
            KeyScalar::Text(String::from(team_id)),
        ));
    }
    if let Some(organization_id) = non_empty(&filters.organization_id) {
        entries.push(Where::Equals(
            KeyColumn::OrganizationId,
            KeyScalar::Text(String::from(organization_id)),
        ));
    }
    entries
}

// creator visibility is scoped to the teams the caller still belongs to when that list is known
fn created_by_arm(user_id: &str, member_team_ids: Option<&[String]>) -> WhereClause {
    let created = vec![Where::Equals(
        KeyColumn::CreatedBy,
        KeyScalar::Text(String::from(user_id)),
    )];
    let Some(ids) = member_team_ids else {
        return created;
    };
    let teams = if ids.is_empty() {
        vec![Where::IsNull(KeyColumn::TeamId)]
    } else {
        vec![Where::Or(vec![
            vec![Where::In(KeyColumn::TeamId, ids.to_vec())],
            vec![Where::IsNull(KeyColumn::TeamId)],
        ])]
    };
    vec![Where::And(vec![created, teams])]
}

// members see the team's service accounts only, and only for teams where they are not admins
fn member_service_account_arm(filters: &KeyFilters) -> Option<WhereClause> {
    let member_team_ids = filters
        .member_team_ids
        .as_deref()
        .filter(|ids| !ids.is_empty())?;
    let admin_team_ids = filters.admin_team_ids.as_deref().unwrap_or(&[]);
    let member_only: Vec<String> = member_team_ids
        .iter()
        .filter(|team_id| !admin_team_ids.contains(*team_id))
        .cloned()
        .collect();
    if member_only.is_empty() {
        return None;
    }
    Some(vec![Where::And(vec![
        vec![Where::In(KeyColumn::TeamId, member_only)],
        vec![Where::IsNull(KeyColumn::UserId)],
    ])])
}
