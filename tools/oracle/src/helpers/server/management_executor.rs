use std::collections::BTreeMap;
use std::future::Future;

use betterlite_server::list_api::memory::{Cell, Cells, InMemoryListExecutor};
use betterlite_server::list_api::plan::types::{
    AnyOf, Compare, ComparisonOp, FacetExecutor as _, FilterValue, IsNull, ListExecutor as _,
    Predicate, QueryPlan, SortKey, Within,
};
use betterlite_server::list_api::sql::{order_by_sql, where_sql};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        let cases = input
            .get("cases")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                Box::new(HelperError {
                    class: "InvalidInput".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: "cases must be an object".to_owned(),
                })
            })?;
        let mut output = serde_json::Map::new();
        for (name, raw_case) in cases {
            output.insert(name.clone(), evaluate(raw_case)?);
        }
        Ok(Value::Object(output))
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Case {
    Sql {
        #[serde(rename = "where", default)]
        predicates: Vec<PredicateInput>,
        #[serde(default = "default_first_index")]
        first_index: usize,
        #[serde(default)]
        order: Vec<SortKey>,
    },
    Memory {
        #[serde(rename = "where", default)]
        predicates: Vec<PredicateInput>,
        #[serde(default)]
        rows: Vec<BTreeMap<String, Value>>,
        #[serde(default)]
        order: Vec<SortKey>,
        distinct_field: Option<String>,
    },
}

const fn default_first_index() -> usize {
    1
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PredicateInput {
    Compare {
        field: String,
        op: ComparisonOp,
        value: Value,
        value_type: Option<String>,
    },
    Within {
        field: String,
        values: Vec<Value>,
    },
    IsNull {
        field: String,
        #[serde(default)]
        negated: bool,
    },
    AnyOf {
        clauses: Vec<CompareInput>,
    },
}

#[derive(Debug, Deserialize)]
struct CompareInput {
    field: String,
    op: ComparisonOp,
    value: Value,
}

fn filter_value(value: &Value, value_type: Option<&str>) -> Result<FilterValue, Box<HelperError>> {
    if value_type == Some("timestamp") {
        return timestamp_filter(value);
    }
    match value {
        Value::String(text) => Ok(FilterValue::Text(text.clone())),
        Value::Number(number) => integer_filter(number),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => {
            Err(Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "unsupported filter value".to_owned(),
            }))
        }
    }
}

fn timestamp_filter(value: &Value) -> Result<FilterValue, Box<HelperError>> {
    let raw = value.as_str().ok_or_else(|| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "timestamp value must be text".to_owned(),
        })
    })?;
    let timestamp = DateTime::parse_from_rfc3339(raw)
        .map_err(|error| {
            Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?
        .with_timezone(&Utc);
    Ok(FilterValue::Timestamp(timestamp))
}

fn integer_filter(number: &serde_json::Number) -> Result<FilterValue, Box<HelperError>> {
    number.as_i64().map(FilterValue::Int).ok_or_else(|| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "number must be an integer".to_owned(),
        })
    })
}

fn predicate(input: PredicateInput) -> Result<Predicate, Box<HelperError>> {
    match input {
        PredicateInput::Compare {
            field,
            op,
            value,
            value_type,
        } => Ok(Predicate::Compare(Compare {
            field,
            op,
            value: filter_value(&value, value_type.as_deref())?,
        })),
        PredicateInput::Within { field, values } => {
            let parsed_values = values
                .iter()
                .map(|value| filter_value(value, None))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Predicate::Within(Within {
                field,
                values: parsed_values,
            }))
        }
        PredicateInput::IsNull { field, negated } => {
            Ok(Predicate::IsNull(IsNull { field, negated }))
        }
        PredicateInput::AnyOf { clauses } => {
            let parsed_clauses = clauses
                .into_iter()
                .map(|clause| {
                    Ok(Compare {
                        field: clause.field,
                        op: clause.op,
                        value: filter_value(&clause.value, None)?,
                    })
                })
                .collect::<Result<Vec<_>, Box<HelperError>>>()?;
            Ok(Predicate::AnyOf(AnyOf {
                clauses: parsed_clauses,
            }))
        }
    }
}

fn cell(value: &Value) -> Result<Cell, Box<HelperError>> {
    match value {
        Value::Null => Ok(Cell::Null),
        Value::String(text) => Ok(Cell::Text(text.clone())),
        Value::Number(number) => number.as_i64().map(Cell::Int).ok_or_else(|| {
            Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "cell number must be an integer".to_owned(),
            })
        }),
        Value::Array(items) => Ok(Cell::Repeated(
            items.iter().map(cell).collect::<Result<Vec<_>, _>>()?,
        )),
        Value::Bool(_) | Value::Object(_) => Err(Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "unsupported cell value".to_owned(),
        })),
    }
}

fn evaluate(raw: &Value) -> Result<Value, Box<HelperError>> {
    let case: Case = serde_json::from_value(raw.clone()).map_err(|error| {
        Box::new(HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    match case {
        Case::Sql {
            predicates,
            first_index,
            order,
        } => evaluate_sql(predicates, first_index, &order),
        Case::Memory {
            predicates,
            rows,
            order,
            distinct_field,
        } => evaluate_memory(predicates, &rows, &order, distinct_field.as_deref()),
    }
}

fn evaluate_sql(
    predicates: Vec<PredicateInput>,
    first_index: usize,
    order: &[SortKey],
) -> Result<Value, Box<HelperError>> {
    let where_ = predicates
        .into_iter()
        .map(predicate)
        .collect::<Result<Vec<_>, _>>()?;
    let (where_sql, params) = where_sql(&where_, first_index).map_err(|error| {
        Box::new(HelperError {
            class: "SqlRenderError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    let bound_params = params.iter().map(filter_json).collect::<Vec<_>>();
    Ok(json!({"where": where_sql, "params": bound_params, "order": order_by_sql(order)}))
}

fn evaluate_memory(
    predicates: Vec<PredicateInput>,
    rows: &[BTreeMap<String, Value>],
    order: &[SortKey],
    distinct_field: Option<&str>,
) -> Result<Value, Box<HelperError>> {
    let where_ = predicates
        .into_iter()
        .map(predicate)
        .collect::<Result<Vec<_>, _>>()?;
    let row_cells = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|(name, value)| Ok((name.clone(), cell(value)?)))
                .collect::<Result<Cells, Box<HelperError>>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let executor = InMemoryListExecutor {
        rows: rows.iter().cloned().zip(row_cells).collect(),
    };
    block_on(async {
        let count = executor.count(&where_).await;
        let plan = QueryPlan {
            where_,
            order: order.to_vec(),
            skip: 0,
            take: u64::try_from(rows.len()).unwrap_or(u64::MAX),
        };
        let selected = executor.find_many(&plan).await;
        let distinct = match distinct_field {
            Some(field) => executor.distinct(field, &plan.where_).await,
            None => Vec::new(),
        };
        Ok(json!({"count": count, "rows": selected, "distinct": distinct}))
    })?
}

fn filter_json(value: &FilterValue) -> Value {
    match value {
        FilterValue::Text(text) => Value::String(text.clone()),
        FilterValue::Int(number) => json!(number),
        FilterValue::Money(amount) => json!(amount.to_string()),
        FilterValue::Timestamp(timestamp) => json!(timestamp.to_rfc3339()),
    }
}

fn block_on<F>(future: F) -> Result<F::Output, Box<HelperError>>
where
    F: Future,
{
    tokio::runtime::Builder::new_current_thread()
        .build()
        .map(|runtime| runtime.block_on(future))
        .map_err(|error| {
            Box::new(HelperError {
                class: "RuntimeError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: (error).to_string(),
            })
        })
}
