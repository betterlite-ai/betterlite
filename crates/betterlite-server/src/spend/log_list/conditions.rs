use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::authz::{Authorization, ScopeClause};
use crate::list_api::problem::escape_like;
use crate::spend::log_query::SpendLogQuery;
use crate::spend::log_query::prepare::PreparedSpendLogQuery;
use crate::spend::log_query::sql::{
    REPRESENTATIVE_ORDER_SQL, SESSION_GROUP_KEY_SQL, SESSION_KEY_EXPR,
    internal_health_check_api_keys, spend_log_search_condition,
};
use crate::spend::log_query::validate::{CacheHitFilter, SpendLogChoices};
use crate::spend::sessions::grouped::sql::where_clause;
use crate::spend::sessions::params::SessionParam;

const SUCCESS_STATUS: &str = "success";

pub struct ConditionInputs<'query> {
    pub query: &'query SpendLogQuery,
    pub prepared: &'query PreparedSpendLogQuery,
    pub choices: &'query SpendLogChoices,
    pub authorization: &'query Authorization,
}

pub struct BuiltConditions {
    pub conditions: Vec<String>,
    pub params: Vec<SessionParam>,
    // the placeholder the ORDER BY repeats to put an exact request_id match first
    pub request_id_index: Option<usize>,
}

// the conditions the list assembles after authorization, in the source's order, every value bound
#[must_use]
pub fn build_conditions(inputs: &ConditionInputs<'_>) -> BuiltConditions {
    let mut built = BuiltConditions {
        conditions: Vec::new(),
        params: Vec::new(),
        request_id_index: None,
    };
    push_window(&mut built, inputs.prepared);
    push_search(&mut built, inputs.prepared);
    push_equality(&mut built, inputs.prepared);
    push_request_id(&mut built, inputs.prepared);
    push_scope(&mut built, &inputs.authorization.scope);
    // with group_by_session and no search the status is decided by the session's representative, not by every member row
    let status_deferred = inputs.query.filters.status_filter.is_some()
        && inputs.query.flags.group_by_session
        && inputs.query.filters.search.is_none();
    push_session_and_status(&mut built, inputs.query, status_deferred);
    push_cache_and_span(&mut built, inputs.choices);
    push_health_checks(&mut built, inputs.query);
    push_spend_bounds(&mut built, inputs.prepared);
    push_metadata_and_oauth(&mut built, inputs.prepared, inputs.query);
    if status_deferred {
        push_grouped_status(&mut built, inputs.query);
    }
    built
}

fn push_moment(built: &mut BuiltConditions, prefix: &str, moment: DateTime<Utc>) {
    let index = built.params.len().saturating_add(1);
    built.conditions.push(format!(
        "{prefix} (${index}::timestamptz AT TIME ZONE 'UTC')"
    ));
    built.params.push(SessionParam::Text(moment.to_rfc3339()));
}

// a UI search owns its window, so the outer inclusive window is skipped there and the search carries its own bounds
fn push_window(built: &mut BuiltConditions, prepared: &PreparedSpendLogQuery) {
    if prepared.search_owns_window {
        return;
    }
    let Some(window) = &prepared.window else {
        return;
    };
    push_moment(built, "\"startTime\" >=", window.start_inclusive);
    push_moment(built, "\"startTime\" <=", window.end_inclusive);
}

fn push_search(built: &mut BuiltConditions, prepared: &PreparedSpendLogQuery) {
    let Some(search) = &prepared.search else {
        return;
    };
    let index = built.params.len().saturating_add(1);
    let condition = spend_log_search_condition(
        &search.value,
        search.window.start_inclusive,
        search.window.end_inclusive,
        index,
    );
    built.conditions.push(condition.sql);
    let (value, start, end) = condition.params;
    built.params.push(SessionParam::Text(value));
    built.params.push(SessionParam::Text(start.to_rfc3339()));
    built.params.push(SessionParam::Text(end.to_rfc3339()));
}

fn push_equality(built: &mut BuiltConditions, prepared: &PreparedSpendLogQuery) {
    for filter in &prepared.equality {
        let index = built.params.len().saturating_add(1);
        built
            .conditions
            .push(format!("{} = ${index}", filter.column.sql_column()));
        built.params.push(SessionParam::Text(filter.value.clone()));
    }
}

fn push_request_id(built: &mut BuiltConditions, prepared: &PreparedSpendLogQuery) {
    let Some(request_id) = &prepared.request_id else {
        return;
    };
    let index = built.params.len().saturating_add(1);
    built.conditions.push(format!(
        "(request_id = ${index} OR litellm_call_id = ${index})"
    ));
    built.params.push(SessionParam::Text(request_id.clone()));
    built.request_id_index = Some(index);
}

fn push_scope(built: &mut BuiltConditions, scope: &ScopeClause) {
    match scope {
        ScopeClause::None => {}
        ScopeClause::OwnUser { user_id } => {
            let index = built.params.len().saturating_add(1);
            built.conditions.push(format!("\"user\" = ${index}"));
            built.params.push(SessionParam::MaybeText(user_id.clone()));
        }
        ScopeClause::OwnUserOrTeams { user_id, team_ids } => {
            let user_index = built.params.len().saturating_add(1);
            let teams_index = user_index.saturating_add(1);
            built.conditions.push(format!(
                "(\"user\" = ${user_index} OR team_id = ANY(${teams_index}::text[]))"
            ));
            built.params.push(SessionParam::MaybeText(user_id.clone()));
            built.params.push(SessionParam::Texts(team_ids.clone()));
        }
    }
}

// session_id is a literal match escaped through the shared list-API helper; a success status also matches NULL, and a deferred one waits for the representative
fn push_session_and_status(
    built: &mut BuiltConditions,
    query: &SpendLogQuery,
    status_deferred: bool,
) {
    if let Some(session_id) = &query.filters.session_id {
        let index = built.params.len().saturating_add(1);
        built.conditions.push(format!("session_id LIKE ${index}"));
        built
            .params
            .push(SessionParam::Text(format!("%{}%", escape_like(session_id))));
    }
    if status_deferred {
        return;
    }
    let Some(status) = query.filters.status_filter.as_deref() else {
        return;
    };
    if status == SUCCESS_STATUS {
        built
            .conditions
            .push(String::from("(status = 'success' OR status IS NULL)"));
        return;
    }
    let index = built.params.len().saturating_add(1);
    built.conditions.push(format!("status = ${index}"));
    built.params.push(SessionParam::Text(status.to_owned()));
}

fn push_cache_and_span(built: &mut BuiltConditions, choices: &SpendLogChoices) {
    match choices.cache_hit {
        Some(CacheHitFilter::Hit) => built
            .conditions
            .push(String::from("LOWER(cache_hit) = 'true'")),
        Some(CacheHitFilter::Miss) => built.conditions.push(String::from(
            "(cache_hit IS NULL OR LOWER(cache_hit) != 'true')",
        )),
        None => {}
    }
    if let Some(condition) = choices.span_condition {
        built.conditions.push(condition.to_owned());
    }
}

fn push_health_checks(built: &mut BuiltConditions, query: &SpendLogQuery) {
    if !query.flags.exclude_internal_health_checks {
        return;
    }
    let [name, hashed] = internal_health_check_api_keys();
    let name_index = built.params.len().saturating_add(1);
    let hashed_index = name_index.saturating_add(1);
    built
        .conditions
        .push(format!("api_key NOT IN (${name_index}, ${hashed_index})"));
    built.params.push(SessionParam::Text(name));
    built.params.push(SessionParam::Text(hashed));
}

// the stored column is a double, so the bound is cast to one instead of compared as a decimal
fn push_money(built: &mut BuiltConditions, prefix: &str, value: Decimal) {
    let index = built.params.len().saturating_add(1);
    built
        .conditions
        .push(format!("{prefix} ${index}::double precision"));
    built.params.push(SessionParam::Text(value.to_string()));
}

fn push_spend_bounds(built: &mut BuiltConditions, prepared: &PreparedSpendLogQuery) {
    if let Some(minimum) = prepared.spend.minimum_inclusive {
        push_money(built, "spend >=", minimum);
    }
    if let Some(maximum) = prepared.spend.maximum_inclusive {
        push_money(built, "spend <=", maximum);
    }
}

// metadata LIKE filters keep their wildcards, and json.dumps writes the oauth stamp as lowercase true/false so a row without it matches neither value
fn push_metadata_and_oauth(
    built: &mut BuiltConditions,
    prepared: &PreparedSpendLogQuery,
    query: &SpendLogQuery,
) {
    for filter in &prepared.metadata {
        let index = built.params.len().saturating_add(1);
        built.conditions.push(format!(
            "{} {} ${index}",
            filter.sql_column(),
            filter.operator().sql()
        ));
        built.params.push(SessionParam::Text(filter.parameter()));
    }
    if let Some(flag) = query.flags.used_client_oauth_token {
        let index = built.params.len().saturating_add(1);
        built
            .conditions
            .push(format!("metadata->>'used_client_oauth_token' = ${index}"));
        let text = if flag { "true" } else { "false" };
        built.params.push(SessionParam::Text(text.to_owned()));
    }
}

// the deferred status keeps the outcome of each session's representative, chosen after every other condition narrowed its rows
fn push_grouped_status(built: &mut BuiltConditions, query: &SpendLogQuery) {
    let Some(status) = query.filters.status_filter.as_deref() else {
        return;
    };
    let joined = where_clause(&built.conditions);
    let index = built.params.len().saturating_add(1);
    built.conditions.push(format!(
        "({SESSION_GROUP_KEY_SQL}) IN (SELECT session_key, api_key FROM (SELECT DISTINCT ON ({SESSION_GROUP_KEY_SQL}) {SESSION_KEY_EXPR} AS session_key, api_key, status FROM \"LiteLLM_SpendLogs\" WHERE {joined} ORDER BY {SESSION_GROUP_KEY_SQL}, {REPRESENTATIVE_ORDER_SQL}) AS session_outcomes WHERE COALESCE(status, 'success') = ${index})"
    ));
    built.params.push(SessionParam::Text(status.to_owned()));
}
