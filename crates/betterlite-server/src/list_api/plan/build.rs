use std::collections::{BTreeMap, BTreeSet};

use crate::list_api::plan::coerce::{
    allowed_params, is_known_param, parse_filters, parse_page, parse_page_size, parse_sort,
    search_predicate,
};
use crate::list_api::plan::types::{ListSpec, Predicate, QueryPlan, Scope, SortKey};
use crate::list_api::problem;
use crate::management_envelopes::ProblemDetail;

fn scope_predicates(scope: &Scope) -> Result<Vec<Predicate>, ProblemDetail> {
    match scope {
        Scope::All => Ok(Vec::new()),
        Scope::Where(predicates) => Ok(predicates.clone()),
        Scope::Denied(reason) => Err(ProblemDetail {
            r#type: "urn:litellm:error:forbidden".to_owned(),
            title: "Forbidden".to_owned(),
            status: 403,
            detail: reason.clone(),
            allowed: None,
        }),
    }
}

fn unknown_params(spec: &ListSpec, params: &BTreeMap<String, String>) -> Option<ProblemDetail> {
    let unknown: BTreeSet<&str> = params
        .keys()
        .map(String::as_str)
        .filter(|name| !is_known_param(spec, name))
        .collect();
    (!unknown.is_empty()).then(|| problem::unknown_query_parameter(&unknown, allowed_params(spec)))
}

pub fn build_query_plan(
    spec: &ListSpec,
    params: &BTreeMap<String, String>,
    scope: &Scope,
) -> Result<QueryPlan, ProblemDetail> {
    let scope_predicates = scope_predicates(scope)?;
    if let Some(problem) = unknown_params(spec, params) {
        return Err(problem);
    }
    let page = parse_page(params)?;
    let page_size = parse_page_size(spec, params)?;
    let sort = parse_sort(spec, params)?;
    let filters = parse_filters(spec, params)?;
    let search = search_predicate(spec, params);

    let mut where_ = scope_predicates;
    where_.extend(filters);
    where_.extend(search);

    let mut order = sort;
    order.push(SortKey {
        field: spec.tiebreaker.clone(),
        descending: false,
    });

    let skip = page
        .checked_sub(1)
        .and_then(|pages_before| pages_before.checked_mul(page_size))
        .ok_or_else(|| {
            problem::invalid_query_parameter(
                "'page' and 'page_size' overflow the offset.".to_owned(),
            )
        })?;

    Ok(QueryPlan {
        where_,
        order,
        skip,
        take: page_size,
    })
}

pub fn facet_where(
    spec: &ListSpec,
    params: &BTreeMap<String, String>,
    scope: &Scope,
) -> Result<Vec<Predicate>, ProblemDetail> {
    let scope_predicates = scope_predicates(scope)?;
    let filters = parse_filters(spec, params)?;
    let search = search_predicate(spec, params);

    let mut where_ = scope_predicates;
    where_.extend(filters);
    where_.extend(search);
    Ok(where_)
}
