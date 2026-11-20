use crate::key_query_plan::filters::{
    KeyColumn, KeyFilters, KeyPredicate, KeyScalar, WhereClause, WhereEntry, matching_predicate,
    truthy,
};

// every branch LiteLLM ORs together before the global filters narrow them again
#[must_use]
pub fn visibility_arms(filters: &KeyFilters) -> Vec<WhereClause> {
    let mut arms = Vec::new();
    let own = own_arm(filters);
    if !own.0.is_empty() {
        arms.push(own);
    }
    if filters.include_created_by_keys
        && let Some(user_id) = truthy(&filters.user_id)
    {
        arms.push(created_by_arm(user_id, filters.member_team_ids.as_deref()));
    }
    if let Some(admin_team_ids) = member_list(&filters.admin_team_ids) {
        arms.push(WhereClause(vec![WhereEntry::Predicate(KeyPredicate::In(
            KeyColumn::TeamId,
            admin_team_ids.to_vec(),
        ))]));
    }
    if let Some(arm) = member_service_account_arm(filters) {
        arms.push(arm);
    }
    arms
}

// the caller's own keys; exclude_team_id and organization_id belong to this arm alone
fn own_arm(filters: &KeyFilters) -> WhereClause {
    let mut entries = Vec::new();
    if let Some(user_id) = truthy(&filters.user_id) {
        entries.push(WhereEntry::Predicate(matching_predicate(
            KeyColumn::UserId,
            filters.user_matching,
            user_id,
        )));
    }
    if let Some(team_id) = truthy(&filters.exclude_team_id) {
        entries.push(WhereEntry::Predicate(KeyPredicate::NotEquals(
            KeyColumn::TeamId,
            KeyScalar::Text(String::from(team_id)),
        )));
    }
    if let Some(organization_id) = truthy(&filters.organization_id) {
        entries.push(WhereEntry::Predicate(KeyPredicate::Equals(
            KeyColumn::OrganizationId,
            KeyScalar::Text(String::from(organization_id)),
        )));
    }
    WhereClause(entries)
}

// creator visibility is scoped to the teams the caller still belongs to when that list is known
fn created_by_arm(user_id: &str, member_team_ids: Option<&[String]>) -> WhereClause {
    let created = WhereClause(vec![WhereEntry::Predicate(KeyPredicate::Equals(
        KeyColumn::CreatedBy,
        KeyScalar::Text(String::from(user_id)),
    ))]);
    match member_team_ids {
        None => created,
        Some(ids) if ids.is_empty() => WhereClause(vec![WhereEntry::And(vec![
            created,
            WhereClause(vec![WhereEntry::Predicate(KeyPredicate::IsNull(
                KeyColumn::TeamId,
            ))]),
        ])]),
        Some(ids) => {
            let in_arm = WhereClause(vec![WhereEntry::Predicate(KeyPredicate::In(
                KeyColumn::TeamId,
                ids.to_vec(),
            ))]);
            let null_arm = WhereClause(vec![WhereEntry::Predicate(KeyPredicate::IsNull(
                KeyColumn::TeamId,
            ))]);
            WhereClause(vec![WhereEntry::And(vec![
                created,
                WhereClause(vec![WhereEntry::Or(vec![in_arm, null_arm])]),
            ])])
        }
    }
}

// members see the team's service accounts only, and only for teams where they are not admins
fn member_service_account_arm(filters: &KeyFilters) -> Option<WhereClause> {
    let member_team_ids = member_list(&filters.member_team_ids)?;
    let admin_team_ids = filters.admin_team_ids.as_deref().unwrap_or(&[]);
    let member_only: Vec<String> = member_team_ids
        .iter()
        .filter(|team_id| !admin_team_ids.iter().any(|admin| admin == *team_id))
        .cloned()
        .collect();
    if member_only.is_empty() {
        return None;
    }
    Some(WhereClause(vec![WhereEntry::And(vec![
        WhereClause(vec![WhereEntry::Predicate(KeyPredicate::In(
            KeyColumn::TeamId,
            member_only,
        ))]),
        WhereClause(vec![WhereEntry::Predicate(KeyPredicate::IsNull(
            KeyColumn::UserId,
        ))]),
    ])]))
}

fn member_list(team_ids: &Option<Vec<String>>) -> Option<&[String]> {
    match team_ids {
        Some(ids) if !ids.is_empty() => Some(ids.as_slice()),
        Some(_) | None => None,
    }
}
