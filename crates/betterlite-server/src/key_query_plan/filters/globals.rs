use chrono::{DateTime, Utc};

use crate::key_query_plan::filters::visibility::visibility_arms;
use crate::key_query_plan::filters::{
    KeyColumn, KeyFilters, KeyScalar, Where, WhereClause, build_key_search_where,
    filter_out_ui_session_tokens, non_empty,
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
        // one arm merges into the top-level object instead of adding an OR node
        1 => [ui, arms.pop().unwrap_or_default()].concat(),
        _ => vec![Where::And(vec![ui, vec![Where::Or(arms)]])],
    };
    let globals = global_filters(filters, now);
    if globals.is_empty() {
        return combined;
    }
    let mut conditions = vec![combined];
    conditions.extend(globals);
    vec![Where::And(conditions)]
}

fn global_filters(filters: &KeyFilters, now: DateTime<Utc>) -> Vec<WhereClause> {
    let mut globals = text_globals(filters);
    let expires = filters
        .expires_filter
        .as_deref()
        .filter(|value| VALID_EXPIRES_FILTER_VALUES.contains(value));
    globals.extend(expires.map(|filter| build_expires_where_clause(filter, now)));
    globals.extend(build_status_where_clause(
        filters.status_filter.as_deref(),
        now,
    ));
    globals
}

// the order of these clauses is the order LiteLLM's AND array holds
fn text_globals(filters: &KeyFilters) -> Vec<WhereClause> {
    [
        non_empty(&filters.key_alias).map(|alias| {
            let matching = Where::matching(KeyColumn::KeyAlias, filters.key_alias_matching, alias);
            vec![matching]
        }),
        equals_text(KeyColumn::Token, filters.key_hash.as_deref()),
        non_empty(&filters.search).map(build_key_search_where),
        equals_text(KeyColumn::TeamId, filters.team_id.as_deref()),
        equals_text(KeyColumn::ProjectId, filters.project_id.as_deref()),
        non_empty(&filters.access_group_id).map(|group| {
            let values = vec![String::from(group)];
            vec![Where::HasSome(KeyColumn::AccessGroupIds, values)]
        }),
        equals_text(KeyColumn::AgentId, filters.agent_id.as_deref()),
    ]
    .into_iter()
    .flatten()
    .collect()
}

// an absent or empty value skips the filter, as it does everywhere else in the builder
fn equals_text(column: KeyColumn, value: Option<&str>) -> Option<WhereClause> {
    let text = value.filter(|text| !text.is_empty())?;
    Some(vec![Where::Equals(
        column,
        KeyScalar::Text(String::from(text)),
    )])
}
