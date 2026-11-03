#[path = "coerce/numeric.rs"]
pub(in crate::list_api::plan) mod numeric;

use std::collections::{BTreeMap, BTreeSet};

use crate::list_api::plan::params::parse_filter_key;
use crate::list_api::plan::types::{
    AnyOf, Compare, ComparisonOp, FilterOp, FilterSpec, FilterType, FilterValue, IsNull, ListSpec,
    Predicate, SEARCH_PARAM, SORT_PARAM, SortKey, Within,
};
use crate::list_api::problem;
use crate::management_envelopes::ProblemDetail;

pub fn parse_sort(
    spec: &ListSpec,
    params: &BTreeMap<String, String>,
) -> Result<Vec<SortKey>, ProblemDetail> {
    let Some(raw) = params.get(SORT_PARAM) else {
        return Ok(spec.default_sort.clone());
    };
    let keys: Vec<SortKey> = raw
        .split(',')
        .map(str::trim)
        .map(|segment| SortKey {
            field: segment.strip_prefix('-').unwrap_or(segment).to_owned(),
            descending: segment.starts_with('-'),
        })
        .collect();
    let rejected: BTreeSet<&str> = keys
        .iter()
        .map(|key| key.field.as_str())
        .filter(|field| !spec.sortable.contains(*field))
        .collect();
    if !rejected.is_empty() {
        return Err(problem::sort_problem(
            problem::SortProblem::Invalid {
                resource: &spec.resource,
            },
            &rejected,
            &spec.sortable,
        ));
    }
    let fields: Vec<&str> = keys.iter().map(|key| key.field.as_str()).collect();
    let repeated: BTreeSet<&str> = fields
        .iter()
        .filter(|field| fields.iter().filter(|other| other == field).count() > 1)
        .copied()
        .collect();
    if !repeated.is_empty() {
        return Err(problem::sort_problem(
            problem::SortProblem::Duplicate,
            &repeated,
            &spec.sortable,
        ));
    }
    Ok(keys)
}

pub fn coerce(
    field: &str,
    op: FilterOp,
    raw: &str,
    target: FilterType,
) -> Result<FilterValue, ProblemDetail> {
    let invalid = || {
        problem::invalid_query_parameter(format!(
            "'filter[{field}][{}]' is not a valid {}: {raw:?}.",
            op.as_wire_name(),
            target_name(target)
        ))
    };
    match target {
        FilterType::Text => Ok(FilterValue::Text(raw.to_owned())),
        FilterType::Int => numeric::parse_integer_filter(raw)
            .map(FilterValue::Int)
            .ok_or_else(invalid),
        FilterType::Money => numeric::parse_decimal(raw)
            .map(FilterValue::Money)
            .ok_or_else(invalid),
        FilterType::Timestamp => numeric::parse_timestamp(raw)
            .map(FilterValue::Timestamp)
            .ok_or_else(invalid),
    }
}

const fn target_name(target: FilterType) -> &'static str {
    match target {
        FilterType::Text => "str",
        FilterType::Int => "int",
        FilterType::Money => "Decimal",
        FilterType::Timestamp => "datetime",
    }
}

fn parse_within_filter(
    field: &str,
    raw: &str,
    value_type: FilterType,
) -> Result<Predicate, ProblemDetail> {
    let mut values = Vec::new();
    for item in raw.split(',') {
        values.push(coerce(field, FilterOp::In, item.trim(), value_type)?);
    }
    Ok(Predicate::Within(Within {
        field: field.to_owned(),
        values,
    }))
}

fn parse_filter(
    field: &str,
    op: FilterOp,
    raw: &str,
    filter_spec: &FilterSpec,
) -> Result<Predicate, ProblemDetail> {
    if !filter_spec.ops.contains(&op) {
        return Err(problem::unsupported_filter_operator(
            field,
            op,
            &filter_spec.ops,
        ));
    }
    match op {
        FilterOp::Compare(comparison) => {
            coerce(field, op, raw, filter_spec.value_type).map(|value| {
                Predicate::Compare(Compare {
                    field: field.to_owned(),
                    op: comparison,
                    value,
                })
            })
        }
        FilterOp::In => parse_within_filter(field, raw, filter_spec.value_type),
        FilterOp::IsNull => raw
            .to_lowercase()
            .parse::<bool>()
            .map(|is_null| {
                Predicate::IsNull(IsNull {
                    field: field.to_owned(),
                    negated: !is_null,
                })
            })
            .map_err(|_invalid_bool| {
                problem::invalid_query_parameter(format!(
                    "'filter[{field}][is_null]' must be 'true' or 'false'."
                ))
            }),
    }
}

pub fn parse_filters(
    spec: &ListSpec,
    params: &BTreeMap<String, String>,
) -> Result<Vec<Predicate>, ProblemDetail> {
    let mut filters = Vec::new();
    for (name, raw) in params {
        let Some((field, op)) = parse_filter_key(name) else {
            continue;
        };
        let Some(filter_spec) = spec.filters.get(field) else {
            continue;
        };
        filters.push(parse_filter(field, op, raw, filter_spec)?);
    }
    Ok(filters)
}

#[must_use]
pub fn search_predicate(spec: &ListSpec, params: &BTreeMap<String, String>) -> Option<Predicate> {
    let raw = params.get(SEARCH_PARAM)?;
    if raw.is_empty() {
        return None;
    }
    Some(Predicate::AnyOf(AnyOf {
        clauses: spec
            .searchable
            .iter()
            .map(|field| Compare {
                field: field.clone(),
                op: ComparisonOp::Contains,
                value: FilterValue::Text(raw.clone()),
            })
            .collect(),
    }))
}
