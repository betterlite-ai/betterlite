use std::collections::BTreeMap;

use betterlite_server::list_api::memory::Cell;
use betterlite_server::list_api::plan::types::{
    AnyOf, Compare, ComparisonOp, FilterValue, IsNull, Predicate, SortKey, Within,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(super) cases: BTreeMap<String, Case>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub(super) enum CellInput {
    Repeated(Vec<Self>),
    Null,
    Text(String),
    Int(i64),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub(super) enum ValueInput {
    Text(String),
    Int(i64),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(super) enum Case {
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
        rows: Vec<BTreeMap<String, CellInput>>,
        #[serde(default)]
        order: Vec<SortKey>,
        distinct_field: Option<String>,
    },
}

const fn default_first_index() -> usize {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum PredicateInput {
    Compare {
        field: String,
        op: ComparisonOp,
        value: ValueInput,
        value_type: Option<String>,
    },
    Within {
        field: String,
        values: Vec<ValueInput>,
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

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CompareInput {
    field: String,
    op: ComparisonOp,
    value: ValueInput,
}

pub(super) fn predicate(
    input: PredicateInput,
) -> Result<Predicate, Box<crate::helpers::HelperError>> {
    match input {
        PredicateInput::Compare {
            field,
            op,
            value,
            value_type,
        } => Ok(Predicate::Compare(Compare {
            field,
            op,
            value: filter_value(value, value_type.as_deref())?,
        })),
        PredicateInput::Within { field, values } => {
            let parsed_values = values
                .into_iter()
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
                        value: filter_value(clause.value, None)?,
                    })
                })
                .collect::<Result<Vec<_>, Box<crate::helpers::HelperError>>>()?;
            Ok(Predicate::AnyOf(AnyOf {
                clauses: parsed_clauses,
            }))
        }
    }
}

fn filter_value(
    value: ValueInput,
    value_type: Option<&str>,
) -> Result<FilterValue, Box<crate::helpers::HelperError>> {
    if value_type == Some("timestamp") {
        return timestamp_filter(value);
    }
    match value {
        ValueInput::Text(text) => Ok(FilterValue::Text(text)),
        ValueInput::Int(number) => Ok(FilterValue::Int(number)),
    }
}

fn timestamp_filter(value: ValueInput) -> Result<FilterValue, Box<crate::helpers::HelperError>> {
    let ValueInput::Text(raw) = value else {
        return Err(helper_error!(
            "InvalidInput",
            "timestamp value must be text".to_owned()
        ));
    };
    let timestamp = DateTime::parse_from_rfc3339(&raw)
        .map_err(|error| helper_error!("InvalidInput", error.to_string()))?
        .with_timezone(&Utc);
    Ok(FilterValue::Timestamp(timestamp))
}

pub(super) fn cell(value: &CellInput) -> Cell {
    match value {
        CellInput::Repeated(items) => Cell::Repeated(items.iter().map(cell).collect()),
        CellInput::Null => Cell::Null,
        CellInput::Text(text) => Cell::Text(text.clone()),
        CellInput::Int(number) => Cell::Int(*number),
    }
}
