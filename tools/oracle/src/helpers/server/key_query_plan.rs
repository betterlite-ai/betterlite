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

// the builder's own arguments, plus the two flags that pick the matching mode
#[derive(Deserialize)]
struct FilterArgs {
    #[serde(flatten)]
    filters: KeyFilters,
    use_substring_matching: Option<bool>,
    use_key_alias_substring_matching: Option<bool>,
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
        Value::String(sort.order.sql().to_lowercase()),
    );
    Ok(Value::Object(entry))
}

fn filters_answer(args: &FilterArgs, now: &str) -> Result<Value, Box<HelperError>> {
    let instant = parse_now(now)?;
    let mut filters = args.filters.clone();
    filters.user_matching = matching(args.use_substring_matching.unwrap_or(false));
    filters.key_alias_matching = matching(args.use_key_alias_substring_matching.unwrap_or(false));
    Ok(clause_value(&build_key_filter_conditions(
        &filters, instant,
    )))
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
