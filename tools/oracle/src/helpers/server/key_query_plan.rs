use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use betterlite_db::identity_rows::key::Expiry;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_query_plan::filters::globals::build_key_filter_conditions;
use betterlite_server::key_query_plan::filters::{
    KeyFilters, KeyScalar, TextMatching, Where, WhereClause,
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
        filters: Box<FilterArgs>,
    },
}

// the builder's own arguments, plus the two flags that pick the matching mode
#[derive(Deserialize)]
pub(crate) struct FilterArgs {
    #[serde(flatten)]
    filters: KeyFilters,
    use_substring_matching: Option<bool>,
    #[serde(rename = "use_key_alias_substring_matching")]
    alias_substring: Option<bool>,
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
    serde_json::to_value(status).map_err(|error| failure("ValueError", None, error.to_string()))
}

fn status_clause_answer(status_filter: Option<&str>, now: &str) -> Result<Value, Box<HelperError>> {
    let instant = parse_now(now)?;
    let clause = build_status_where_clause(status_filter, instant);
    Ok(clause.map_or(Value::Null, |where_clause| clause_value(&where_clause)))
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
    let direction = sort.order.sql().to_lowercase();
    let mut entry = Map::new();
    entry.insert(sort.column.wire().to_owned(), Value::String(direction));
    Ok(Value::Object(entry))
}

fn filters_answer(args: &FilterArgs, now: &str) -> Result<Value, Box<HelperError>> {
    let instant = parse_now(now)?;
    let mut filters = args.filters.clone();
    filters.user_matching = matching(args.use_substring_matching.unwrap_or(false));
    filters.key_alias_matching = matching(args.alias_substring.unwrap_or(false));
    Ok(clause_value(&build_key_filter_conditions(
        &filters, instant,
    )))
}

const fn matching(substring: bool) -> TextMatching {
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
    for entry in clause {
        let (key, value) = entry_value(entry);
        fields.insert(key, value);
    }
    Value::Object(fields)
}

fn entry_value(entry: &Where) -> (String, Value) {
    match entry {
        Where::And(children) => (
            String::from("AND"),
            Value::Array(children.iter().map(clause_value).collect()),
        ),
        Where::Or(children) => (
            String::from("OR"),
            Value::Array(children.iter().map(clause_value).collect()),
        ),
        Where::IsNull(column) => (column.wire().to_owned(), Value::Null),
        Where::IsNotNull(column) => (column.wire().to_owned(), json!({"not": null})),
        Where::Equals(column, scalar) => (column.wire().to_owned(), scalar_value(scalar)),
        Where::NotEquals(column, scalar) => (
            column.wire().to_owned(),
            json!({"not": scalar_value(scalar)}),
        ),
        Where::Before(column, instant) => (
            column.wire().to_owned(),
            json!({"lt": instant.to_rfc3339_opts(SecondsFormat::AutoSi, true)}),
        ),
        Where::AtOrAfter(column, instant) => (
            column.wire().to_owned(),
            json!({"gte": instant.to_rfc3339_opts(SecondsFormat::AutoSi, true)}),
        ),
        Where::ContainsInsensitive(column, text) => (
            column.wire().to_owned(),
            json!({"contains": text, "mode": "insensitive"}),
        ),
        Where::In(column, values) => (column.wire().to_owned(), json!({"in": values})),
        Where::HasSome(column, values) => (column.wire().to_owned(), json!({"hasSome": values})),
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
