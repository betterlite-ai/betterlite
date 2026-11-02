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
    value_type: FilterType,
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

fn make_spec(input: SpecInput) -> Result<ListSpec, Box<crate::helpers::HelperError>> {
    let mut filters = BTreeMap::new();
    for (field, filter) in input.filters {
        let value_type = filter.value_type;
        let ops = filter
            .ops
            .iter()
            .filter_map(|op| FilterOp::from_wire_name(op))
            .collect();
        filters.insert(field, FilterSpec { value_type, ops });
    }
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

fn comparison_from_wire(op: &str) -> Result<ComparisonOp, Box<crate::helpers::HelperError>> {
    match op {
        "eq" => Ok(ComparisonOp::Eq),
        "contains" => Ok(ComparisonOp::Contains),
        "gte" => Ok(ComparisonOp::Gte),
        other => Err(helper_error(format!("unknown comparison {other}"))),
    }
}

fn loc_part(location: Value) -> Result<LocPart, Box<crate::helpers::HelperError>> {
    if let Some(field) = location.as_str() {
        return Ok(LocPart::Field(field.to_owned()));
    }
    let Some(index) = location.as_u64() else {
        return Err(helper_error("invalid error location".to_owned()));
    };
    let index = usize::try_from(index)
        .map_err(|_overflow| helper_error("invalid error index".to_owned()))?;
    Ok(LocPart::Index(index))
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
            json!({"kind":"isnull", "field":null_predicate.field, "negated":null_predicate.negated})
        }
        Predicate::Within(within) => {
            json!({"kind":"within", "field":within.field, "values":within.values.into_iter().map(value_json).collect::<Vec<_>>() })
        }
        Predicate::AnyOf(any) => {
            json!({"kind":"anyof", "clauses":any.clauses.into_iter().map(|compare| json!({"field":compare.field, "op":comparison_name(compare.op), "value":value_json(compare.value)})).collect::<Vec<_>>()})
        }
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Decode { raw } => {
                let decoded = decode_list_query(raw.as_bytes());
                Ok(json!({"pairs": decoded.pairs, "last_value": decoded.last_value}))
            }
            Input::Plan {
                spec,
                params,
                scope,
            } => {
                let scope = match scope.clone() {
                    ScopeInput::All => Scope::All,
                    ScopeInput::Denied { reason } => Scope::Denied(reason),
                    ScopeInput::Where { predicates } => Scope::Where(
                        predicates
                            .into_iter()
                            .map(|predicate| match predicate {
                                PredicateInput::IsNull { field, negated } => Ok(Predicate::IsNull(
                                    betterlite_server::list_api::plan::types::IsNull {
                                        field,
                                        negated,
                                    },
                                )),
                                PredicateInput::Compare { field, op, value } => {
                                    comparison_from_wire(&op).map(|op| {
                                        Predicate::Compare(
                                            betterlite_server::list_api::plan::types::Compare {
                                                field,
                                                op,
                                                value: FilterValue::Text(value),
                                            },
                                        )
                                    })
                                }
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    ),
                };
                let result = build_query_plan(&make_spec(spec.clone())?, params, &scope);
                match result {
                    Ok(plan) => Ok(json!({
                        "where": plan.where_.into_iter().map(predicate_json).collect::<Vec<_>>(),
                        "order": plan.order.into_iter().map(|sort| json!({"field":sort.field, "descending":sort.descending})).collect::<Vec<_>>(),
                        "skip": plan.skip,
                        "take": plan.take
                    })),
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
