use chrono::{DateTime, Utc};

use crate::key_query_plan::filters::visibility::visibility_arms;
use crate::key_query_plan::filters::{
    KeyColumn, KeyFilters, KeyPredicate, KeyScalar, WhereClause, WhereEntry,
    build_key_search_where, filter_out_ui_session_tokens, matching_predicate, truthy,
};
use crate::key_query_plan::status::{
    VALID_EXPIRES_FILTER_VALUES, build_expires_where_clause, build_status_where_clause,
};

// the full Prisma where: UI-session exclusion, the OR of visibility arms, then every global AND
#[must_use]
pub fn build_key_filter_conditions(filters: &KeyFilters, now: DateTime<Utc>) -> WhereClause {
    let ui = filter_out_ui_session_tokens();
    let mut arms = visibility_arms(filters);
    let combined = match arms.len() {
        0 => ui,
        1 => {
            let mut entries = ui.0;
            if let Some(arm) = arms.pop() {
                entries.extend(arm.0);
            }
            WhereClause(entries)
        }
        _ => WhereClause(vec![WhereEntry::And(vec![
            ui,
            WhereClause(vec![WhereEntry::Or(arms)]),
        ])]),
    };
    let globals = global_filters(filters, now);
    if globals.is_empty() {
        return combined;
    }
    let mut children = Vec::with_capacity(globals.len() + 1);
    children.push(combined);
    children.extend(globals);
    WhereClause(vec![WhereEntry::And(children)])
}

fn global_filters(filters: &KeyFilters, now: DateTime<Utc>) -> Vec<WhereClause> {
    let mut globals = text_globals(filters);
    if let Some(expires_filter) = filters
        .expires_filter
        .as_deref()
        .filter(|value| VALID_EXPIRES_FILTER_VALUES.contains(value))
    {
        globals.push(build_expires_where_clause(expires_filter, now));
    }
    if let Some(status) = build_status_where_clause(filters.status_filter.as_deref(), now) {
        globals.push(status);
    }
    globals
}

// the order of these clauses is the order LiteLLM's AND array holds
fn text_globals(filters: &KeyFilters) -> Vec<WhereClause> {
    let mut globals = Vec::new();
    if let Some(key_alias) = truthy(&filters.key_alias) {
        globals.push(WhereClause(vec![WhereEntry::Predicate(
            matching_predicate(KeyColumn::KeyAlias, filters.key_alias_matching, key_alias),
        )]));
    }
    if let Some(key_hash) = truthy(&filters.key_hash) {
        globals.push(equals_text(KeyColumn::Token, key_hash));
    }
    if let Some(search) = truthy(&filters.search) {
        globals.push(build_key_search_where(search));
    }
    if let Some(team_id) = truthy(&filters.team_id) {
        globals.push(equals_text(KeyColumn::TeamId, team_id));
    }
    if let Some(project_id) = truthy(&filters.project_id) {
        globals.push(equals_text(KeyColumn::ProjectId, project_id));
    }
    if let Some(access_group_id) = truthy(&filters.access_group_id) {
        globals.push(WhereClause(vec![WhereEntry::Predicate(
            KeyPredicate::HasSome(
                KeyColumn::AccessGroupIds,
                vec![String::from(access_group_id)],
            ),
        )]));
    }
    if let Some(agent_id) = truthy(&filters.agent_id) {
        globals.push(equals_text(KeyColumn::AgentId, agent_id));
    }
    globals
}

fn equals_text(column: KeyColumn, value: &str) -> WhereClause {
    WhereClause(vec![WhereEntry::Predicate(KeyPredicate::Equals(
        column,
        KeyScalar::Text(String::from(value)),
    ))])
}
