use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use betterlite_db::identity_rows::key::Expiry;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_query_plan::filters::globals::build_key_filter_conditions;
use betterlite_server::key_query_plan::filters::{
    KeyFilters, KeyPredicate, KeyScalar, TextMatching, WhereClause, WhereEntry,
};
use betterlite_server::key_query_plan::sort::validate_sort_params;
use betterlite_server::key_query_plan::status::{
    KeyStatusSource, build_status_where_clause, derive_key_status,
};

// one call per helper invocation, so a failing call's status is compared like any outcome
#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Status {
        blocked: Option<bool>,
        expires: Option<String>,
        now: String,
    },
    StatusClause {
        status_filter: Option<String>,
        now: String,
    },
    Sort {
        sort_by: Option<String>,
        sort_order: String,
    },
    Filters {
        now: String,
        filters: FilterArgs,
    },
}

// the names and defaults mirror the arguments LiteLLM's builder takes
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilterArgs {
    user_id: Option<String>,
    team_id: Option<String>,
    organization_id: Option<String>,
    key_alias: Option<String>,
    key_hash: Option<String>,
    exclude_team_id: Option<String>,
    admin_team_ids: Option<Vec<String>>,
    member_team_ids: Option<Vec<String>>,
    include_created_by_keys: Option<bool>,
    project_id: Option<String>,
    access_group_id: Option<String>,
    agent_id: Option<String>,
    use_substring_matching: Option<bool>,
    use_key_alias_substring_matching: Option<bool>,
    expires_filter: Option<String>,
    search: Option<String>,
    status_filter: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Status {
                blocked,
                expires,
                now,
            } => status_answer(*blocked, expires.as_deref(), now),
            Input::StatusClause { status_filter, now } => {
                status_clause_answer(status_filter.as_deref(), now)
            }
            Input::Sort {
                sort_by,
                sort_order,
            } => sort_answer(sort_by.as_deref(), sort_order),
            Input::Filters { now, filters } => filters_answer(filters, now),
        }
    }
}

fn status_answer(
    blocked: Option<bool>,
    expires: Option<&str>,
    now: &str,
) -> Result<Value, Box<HelperError>> {
    let instant = parse_now(now)?;
    let source = KeyStatusSource {
        blocked,
        expires: expires.map(|text| Expiry::Text(text.to_owned())),
    };
    let status = derive_key_status(&source, instant).map_err(|error| {
        failure(
            "ValueError",
            None,
            format!("the expiry is not a timestamp: {error}"),
        )
    })?;
    let value = serde_json::to_value(status)
        .map_err(|error| failure("ValueError", None, error.to_string()))?;
    Ok(value)
}

fn status_clause_answer(status_filter: Option<&str>, now: &str) -> Result<Value, Box<HelperError>> {
    let instant = parse_now(now)?;
    let clause = build_status_where_clause(status_filter, instant);
    Ok(clause.map_or(Value::Null, |clause| clause_value(&clause)))
}

fn sort_answer(sort_by: Option<&str>, sort_order: &str) -> Result<Value, Box<HelperError>> {
    let validated = validate_sort_params(sort_by, sort_order).map_err(|error| {
        failure(
            "HTTPException",
            Some(400),
            format!("Invalid sort parameters: {error}"),
        )
    })?;
    let Some(sort) = validated else {
        return Ok(Value::Null);
    };
    let mut entry = Map::new();
    entry.insert(
        sort.column.wire().to_owned(),
        Value::String(sort.order.wire().to_owned()),
    );
    Ok(Value::Object(entry))
}

fn filters_answer(filters: &FilterArgs, now: &str) -> Result<Value, Box<HelperError>> {
    let instant = parse_now(now)?;
    let built = KeyFilters {
        user_id: filters.user_id.clone(),
        team_id: filters.team_id.clone(),
        organization_id: filters.organization_id.clone(),
        key_alias: filters.key_alias.clone(),
        key_hash: filters.key_hash.clone(),
        exclude_team_id: filters.exclude_team_id.clone(),
        admin_team_ids: filters.admin_team_ids.clone(),
        member_team_ids: filters.member_team_ids.clone(),
        include_created_by_keys: filters.include_created_by_keys.unwrap_or(false),
        project_id: filters.project_id.clone(),
        access_group_id: filters.access_group_id.clone(),
        agent_id: filters.agent_id.clone(),
        user_matching: matching(filters.use_substring_matching.unwrap_or(false)),
        key_alias_matching: matching(filters.use_key_alias_substring_matching.unwrap_or(false)),
        expires_filter: filters.expires_filter.clone(),
        search: filters.search.clone(),
        status_filter: filters.status_filter.clone(),
    };
    Ok(clause_value(&build_key_filter_conditions(&built, instant)))
}

fn matching(substring: bool) -> TextMatching {
    if substring {
        TextMatching::Substring
    } else {
        TextMatching::Exact
    }
}

fn parse_now(now: &str) -> Result<DateTime<Utc>, Box<HelperError>> {
    DateTime::parse_from_rfc3339(now)
        .map(|instant| instant.with_timezone(&Utc))
        .map_err(|error| {
            failure(
                "ValueError",
                None,
                format!("{now} is not a timestamp: {error}"),
            )
        })
}

fn clause_value(clause: &WhereClause) -> Value {
    let mut fields = Map::new();
    for entry in &clause.0 {
        match entry {
            WhereEntry::Predicate(predicate) => {
                let (column, value) = predicate_value(predicate);
                fields.insert(column.to_owned(), value);
            }
            WhereEntry::And(children) => {
                let clauses = children.iter().map(clause_value).collect();
                fields.insert(String::from("AND"), Value::Array(clauses));
            }
            WhereEntry::Or(children) => {
                let clauses = children.iter().map(clause_value).collect();
                fields.insert(String::from("OR"), Value::Array(clauses));
            }
        }
    }
    Value::Object(fields)
}

fn predicate_value(predicate: &KeyPredicate) -> (&'static str, Value) {
    match predicate {
        KeyPredicate::IsNull(column) => (column.wire(), Value::Null),
        KeyPredicate::IsNotNull(column) => (column.wire(), json!({"not": null})),
        KeyPredicate::Equals(column, scalar) => (column.wire(), scalar_value(scalar)),
        KeyPredicate::NotEquals(column, scalar) => {
            (column.wire(), json!({"not": scalar_value(scalar)}))
        }
        KeyPredicate::Before(column, instant) => {
            let bound = instant.to_rfc3339_opts(SecondsFormat::AutoSi, true);
            (column.wire(), json!({"lt": bound}))
        }
        KeyPredicate::AtOrAfter(column, instant) => {
            let bound = instant.to_rfc3339_opts(SecondsFormat::AutoSi, true);
            (column.wire(), json!({"gte": bound}))
        }
        KeyPredicate::ContainsInsensitive(column, text) => (
            column.wire(),
            json!({"contains": text, "mode": "insensitive"}),
        ),
        KeyPredicate::In(column, values) => (column.wire(), json!({"in": values})),
        KeyPredicate::HasSome(column, values) => (column.wire(), json!({"hasSome": values})),
    }
}

fn scalar_value(scalar: &KeyScalar) -> Value {
    match scalar {
        KeyScalar::Bool(flag) => Value::Bool(*flag),
        KeyScalar::Text(text) => Value::String(text.clone()),
        KeyScalar::Timestamp(instant) => {
            Value::String(instant.to_rfc3339_opts(SecondsFormat::AutoSi, true))
        }
    }
}

fn failure(class: &str, status: Option<u16>, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
