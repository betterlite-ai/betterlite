use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr as _;

use chrono::{DateTime, TimeZone as _, Utc};
use rust_decimal::Decimal;
use speedate::{Date, DateTime as SpeedateDateTime};

use crate::list_api::plan::types::{
    AnyOf, Compare, ComparisonOp, FilterOp, FilterSpec, FilterType, FilterValue, IsNull, ListSpec,
    PAGE_PARAM, PAGE_SIZE_PARAM, Predicate, SEARCH_PARAM, SORT_PARAM, SortKey, Within,
};
use crate::list_api::problem;
use crate::management_envelopes::ProblemDetail;

#[must_use]
pub fn allowed_params(spec: &ListSpec) -> Vec<String> {
    let mut names: Vec<String> = vec![PAGE_PARAM.to_owned(), PAGE_SIZE_PARAM.to_owned()];
    if !spec.sortable.is_empty() {
        names.push(SORT_PARAM.to_owned());
    }
    if !spec.searchable.is_empty() {
        names.push(SEARCH_PARAM.to_owned());
    }
    for (field, filter_spec) in &spec.filters {
        for op in &filter_spec.ops {
            names.push(filter_param_name(field, *op));
        }
    }
    names.sort();
    names
}

#[must_use]
pub fn facet_allowed_params(spec: &ListSpec) -> Vec<String> {
    allowed_params(spec)
        .into_iter()
        .filter(|name| name != SORT_PARAM)
        .collect()
}

fn filter_param_name(field: &str, op: FilterOp) -> String {
    if op == FilterOp::Compare(ComparisonOp::Eq) {
        format!("filter[{field}]")
    } else {
        format!("filter[{field}][{}]", op.as_wire_name())
    }
}

#[must_use]
pub fn parse_filter_key(name: &str) -> Option<(&str, FilterOp)> {
    let inner = name.strip_prefix("filter[")?.strip_suffix(']')?;
    match inner.split_once("][") {
        None => Some((inner, FilterOp::Compare(ComparisonOp::Eq))),
        Some((field, raw_op)) => FilterOp::from_wire_name(raw_op).map(|op| (field, op)),
    }
}

#[must_use]
pub fn is_known_param(spec: &ListSpec, name: &str) -> bool {
    if name == PAGE_PARAM || name == PAGE_SIZE_PARAM {
        return true;
    }
    if name == SORT_PARAM {
        return !spec.sortable.is_empty();
    }
    if name == SEARCH_PARAM {
        return !spec.searchable.is_empty();
    }
    parse_filter_key(name).is_some_and(|(field, _)| spec.filters.contains_key(field))
}

fn parse_python_int(raw: &str) -> Option<i128> {
    let trimmed = raw.trim();
    let (sign, digits) = trimmed.strip_prefix('-').map_or_else(
        || (1_i128, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| (-1_i128, rest),
    );
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') {
        return None;
    }
    let mut magnitude: i128 = 0;
    let mut previous_was_underscore = false;
    for byte in digits.bytes() {
        if byte == b'_' {
            if previous_was_underscore {
                return None;
            }
            previous_was_underscore = true;
            continue;
        }
        previous_was_underscore = false;
        let digit = i128::from(byte.checked_sub(b'0')?);
        if digit > 9 {
            return None;
        }
        magnitude = magnitude.checked_mul(10)?.checked_add(digit)?;
    }
    magnitude.checked_mul(sign)
}

pub fn parse_positive_int(name: &str, raw: &str) -> Result<u64, ProblemDetail> {
    let value = parse_python_int(raw)
        .ok_or_else(|| problem::invalid_query_parameter(format!("'{name}' must be an integer.")))?;
    let bounded = u64::try_from(value).map_err(|_too_large| {
        problem::invalid_query_parameter(format!("'{name}' is too large."))
    })?;
    if bounded < 1 {
        return Err(problem::invalid_query_parameter(format!(
            "'{name}' must be 1 or greater."
        )));
    }
    Ok(bounded)
}

pub fn parse_page(params: &BTreeMap<String, String>) -> Result<u64, ProblemDetail> {
    params
        .get(PAGE_PARAM)
        .map_or(Ok(1), |raw| parse_positive_int(PAGE_PARAM, raw))
}

pub fn parse_page_size(
    spec: &ListSpec,
    params: &BTreeMap<String, String>,
) -> Result<u64, ProblemDetail> {
    match params.get(PAGE_SIZE_PARAM) {
        None => Ok(spec.default_page_size),
        Some(raw) => {
            let value = parse_positive_int(PAGE_SIZE_PARAM, raw)?;
            Ok(value.min(spec.max_page_size))
        }
    }
}

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
        return Err(problem::invalid_sort_field(
            &spec.resource,
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
        return Err(problem::duplicate_sort_field(&repeated, &spec.sortable));
    }
    Ok(keys)
}

fn to_utc(parsed: &SpeedateDateTime) -> Option<DateTime<Utc>> {
    let nanoseconds = parsed.time.microsecond.checked_mul(1_000)?;
    Utc.timestamp_opt(parsed.timestamp_tz(), nanoseconds)
        .single()
}

fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let parsed = SpeedateDateTime::parse_str(raw).or_else(|_| {
        let date = Date::parse_str(raw)?;
        SpeedateDateTime::parse_str(&format!("{date}T00:00:00"))
    });
    to_utc(&parsed.ok()?)
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
        FilterType::Int => parse_python_int(raw)
            .and_then(|value| i64::try_from(value).ok())
            .map(FilterValue::Int)
            .ok_or_else(invalid),
        FilterType::Money => Decimal::from_str(raw)
            .map(FilterValue::Money)
            .map_err(|_not_decimal| invalid()),
        FilterType::Timestamp => parse_timestamp(raw)
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

fn null_predicate(field: &str, raw: &str) -> Result<Predicate, ProblemDetail> {
    match raw.to_lowercase().as_str() {
        "true" => Ok(Predicate::IsNull(IsNull {
            field: field.to_owned(),
            negated: false,
        })),
        "false" => Ok(Predicate::IsNull(IsNull {
            field: field.to_owned(),
            negated: true,
        })),
        _ => Err(problem::invalid_query_parameter(format!(
            "'filter[{field}][is_null]' must be 'true' or 'false'."
        ))),
    }
}

fn within_predicate(
    field: &str,
    raw: &str,
    target: FilterType,
) -> Result<Predicate, ProblemDetail> {
    let values = raw
        .split(',')
        .map(|item| coerce(field, FilterOp::In, item.trim(), target))
        .collect::<Result<Vec<_>, _>>()?;
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
        FilterOp::IsNull => null_predicate(field, raw),
        FilterOp::In => within_predicate(field, raw, filter_spec.value_type),
        FilterOp::Compare(comparison) => {
            coerce(field, op, raw, filter_spec.value_type).map(|value| {
                Predicate::Compare(Compare {
                    field: field.to_owned(),
                    op: comparison,
                    value,
                })
            })
        }
    }
}

pub fn parse_filters(
    spec: &ListSpec,
    params: &BTreeMap<String, String>,
) -> Result<Vec<Predicate>, ProblemDetail> {
    params
        .iter()
        .filter_map(|(name, raw)| parse_filter_key(name).map(|(field, op)| (field, op, raw)))
        .filter_map(|(field, op, raw)| {
            spec.filters
                .get(field)
                .map(|filter_spec| (field, op, raw, filter_spec))
        })
        .map(|(field, op, raw, filter_spec)| parse_filter(field, op, raw, filter_spec))
        .collect()
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
