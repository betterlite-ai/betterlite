use std::collections::{BTreeMap, BTreeSet};

use betterlite_server::list_api::plan::build::build_query_plan;
use betterlite_server::list_api::plan::query::decode_list_query;
use betterlite_server::list_api::plan::types::{
    ComparisonOp, FilterOp, FilterSpec, FilterType, FilterValue, ListSpec, Predicate, Scope,
    SortKey,
};
use betterlite_server::list_api::problem::{
    LocPart, ValidationErrorDetail, request_validation_problem,
};
use betterlite_server::management_envelopes::ProblemDocument;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Plan {
        spec: SpecInput,
        params: BTreeMap<String, String>,
        scope: ScopeInput,
    },
    Decode {
        raw: String,
    },
    Problem {
        errors: Vec<ErrorInput>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SpecInput {
    resource: String,
    sortable: BTreeSet<String>,
    searchable: BTreeSet<String>,
    filters: BTreeMap<String, FilterInput>,
    default_sort: Vec<SortInput>,
    default_page_size: u64,
    max_page_size: u64,
    tiebreaker: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FilterInput {
    value_type: String,
    ops: BTreeSet<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SortInput {
    field: String,
    descending: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ScopeInput {
    All,
    Denied { reason: String },
    Where { predicates: Vec<PredicateInput> },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PredicateInput {
    Compare {
        field: String,
        op: String,
        value: String,
    },
    IsNull {
        field: String,
        negated: bool,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ErrorInput {
    error_type: String,
    loc: Vec<Value>,
    msg: String,
}

pub(crate) struct Op;

fn helper_error(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn filter_type(name: &str) -> Result<FilterType, Box<crate::helpers::HelperError>> {
    match name {
        "text" => Ok(FilterType::Text),
        "int" => Ok(FilterType::Int),
        "money" => Ok(FilterType::Money),
        "timestamp" => Ok(FilterType::Timestamp),
        other => Err(helper_error(format!("unknown filter type {other}"))),
    }
}

fn filter_op(name: &str) -> Option<FilterOp> {
    FilterOp::from_wire_name(name)
}

fn make_spec(input: SpecInput) -> Result<ListSpec, Box<crate::helpers::HelperError>> {
    let filters = input
        .filters
        .into_iter()
        .map(|(field, filter)| {
            let ops = filter.ops.iter().filter_map(|op| filter_op(op)).collect();
            filter_type(&filter.value_type)
                .map(|value_type| (field, FilterSpec { value_type, ops }))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    ListSpec {
        resource: input.resource,
        sortable: input.sortable,
        searchable: input.searchable,
        filters,
        default_sort: input
            .default_sort
            .into_iter()
            .map(|sort| SortKey {
                field: sort.field,
                descending: sort.descending,
            })
            .collect(),
        default_page_size: input.default_page_size,
        max_page_size: input.max_page_size,
        tiebreaker: input.tiebreaker,
    }
    .validate()
    .map_err(|error| helper_error(error.to_string()))
}

fn make_scope(input: ScopeInput) -> Result<Scope, Box<crate::helpers::HelperError>> {
    match input {
        ScopeInput::All => Ok(Scope::All),
        ScopeInput::Denied { reason } => Ok(Scope::Denied(reason)),
        ScopeInput::Where { predicates } => predicates
            .into_iter()
            .map(make_predicate)
            .collect::<Result<Vec<_>, _>>()
            .map(Scope::Where),
    }
}

fn make_predicate(input: PredicateInput) -> Result<Predicate, Box<crate::helpers::HelperError>> {
    match input {
        PredicateInput::IsNull { field, negated } => Ok(Predicate::IsNull(
            betterlite_server::list_api::plan::types::IsNull { field, negated },
        )),
        PredicateInput::Compare { field, op, value } => {
            let comparison = match op.as_str() {
                "eq" => ComparisonOp::Eq,
                "contains" => ComparisonOp::Contains,
                "gte" => ComparisonOp::Gte,
                other => return Err(helper_error(format!("unknown comparison {other}"))),
            };
            Ok(Predicate::Compare(
                betterlite_server::list_api::plan::types::Compare {
                    field,
                    op: comparison,
                    value: FilterValue::Text(value),
                },
            ))
        }
    }
}

fn loc_part(location: Value) -> Result<LocPart, Box<crate::helpers::HelperError>> {
    match location {
        Value::String(field) => Ok(LocPart::Field(field)),
        Value::Number(index) => index
            .as_u64()
            .and_then(|number| usize::try_from(number).ok())
            .map(LocPart::Index)
            .ok_or_else(|| helper_error("invalid error index".to_owned())),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => {
            Err(helper_error("invalid error location".to_owned()))
        }
    }
}

const fn comparison_name(op: ComparisonOp) -> &'static str {
    match op {
        ComparisonOp::Eq => "eq",
        ComparisonOp::Gte => "gte",
        ComparisonOp::Lte => "lte",
        ComparisonOp::Gt => "gt",
        ComparisonOp::Lt => "lt",
        ComparisonOp::Contains => "contains",
        ComparisonOp::Not => "not",
    }
}

fn value_json(filter_value: FilterValue) -> Value {
    match filter_value {
        FilterValue::Text(text) => json!(text),
        FilterValue::Int(number) => json!(number),
        FilterValue::Money(money) => json!(money.to_string()),
        FilterValue::Timestamp(timestamp) => json!(timestamp.to_rfc3339().replace("+00:00", "Z")),
    }
}

fn problem_json(
    problem: &betterlite_server::management_envelopes::ProblemDetail,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    serde_json::to_value(ProblemDocument::from(problem))
        .map_err(|error| helper_error(error.to_string()))
}

fn predicate_json(predicate: Predicate) -> Value {
    match predicate {
        Predicate::Compare(compare) => {
            json!({"kind":"compare", "field":compare.field, "op":comparison_name(compare.op), "value":value_json(compare.value)})
        }
        Predicate::IsNull(null_predicate) => {
            json!({"kind":"is_null", "field":null_predicate.field, "negated":null_predicate.negated})
        }
        Predicate::Within(within) => {
            json!({"kind":"within", "field":within.field, "values":within.values.into_iter().map(value_json).collect::<Vec<_>>() })
        }
        Predicate::AnyOf(any) => {
            json!({"kind":"any_of", "clauses":any.clauses.into_iter().map(|compare| json!({"field":compare.field, "op":comparison_name(compare.op), "value":value_json(compare.value)})).collect::<Vec<_>>()})
        }
    }
}

fn plan_json(plan: betterlite_server::list_api::plan::types::QueryPlan) -> Value {
    json!({"where": plan.where_.into_iter().map(predicate_json).collect::<Vec<_>>(), "order": plan.order.into_iter().map(|sort| json!({"field":sort.field, "descending":sort.descending})).collect::<Vec<_>>(), "skip": plan.skip, "take": plan.take})
}

fn decode_json(raw: &str) -> Value {
    let decoded = decode_list_query(raw.as_bytes());
    json!({"pairs": decoded.pairs, "last_value": decoded.last_value})
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Decode { raw } => Ok(decode_json(raw)),
            Input::Plan {
                spec,
                params,
                scope,
            } => {
                let result = build_query_plan(
                    &make_spec(spec.clone())?,
                    params,
                    &make_scope(scope.clone())?,
                );
                match result {
                    Ok(plan) => Ok(plan_json(plan)),
                    Err(problem) => problem_json(&problem),
                }
            }
            Input::Problem { errors: raw_errors } => {
                let errors = raw_errors
                    .iter()
                    .map(|error| {
                        let loc = error
                            .loc
                            .iter()
                            .cloned()
                            .map(loc_part)
                            .collect::<Result<Vec<_>, _>>()?;
                        Ok(ValidationErrorDetail {
                            error_type: error.error_type.clone(),
                            loc,
                            msg: error.msg.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>, Box<crate::helpers::HelperError>>>()?;
                let problem = request_validation_problem(&errors);
                problem_json(&problem)
            }
        }
    }
}
