use super::links::{build_list_links, build_page_links};
use super::plan::build::{build_query_plan, facet_where};
use super::plan::coerce::numeric::{parse_page, parse_page_size};
use super::plan::params::{facet_allowed_params, is_known_param};
use super::plan::types::{FacetExecutor, ListExecutor, ListSpec, SORT_PARAM, Scope};
use super::problem::{ManagementProblem, unknown_query_parameter};
use crate::management_envelopes::{
    FacetListResponse, ListMeta, ListResponse, PageMeta, ProblemDetail,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub struct ListCall<'request> {
    pub params: &'request BTreeMap<String, String>,
    pub pairs: &'request [(String, String)],
    pub scope: &'request Scope,
    pub path: &'request str,
}

pub struct FacetCall<'request> {
    pub params: &'request BTreeMap<String, String>,
    pub pairs: &'request [(String, String)],
    pub scope: &'request Scope,
    pub path: &'request str,
    pub field: &'request str,
}

mod tests;

fn duplicate_params(pairs: &[(String, String)]) -> Vec<String> {
    let mut counts = BTreeMap::new();
    for (name, _) in pairs {
        let count = counts.entry(name).or_insert(0_u8);
        *count = count.saturating_add(1);
    }
    counts
        .into_iter()
        .filter_map(|(name, count)| (count > 1).then_some(name.clone()))
        .collect()
}

fn duplicate_problem(duplicates: &[String], facet: bool) -> ManagementProblem {
    let list_kind = if facet {
        "filter values"
    } else {
        "sort keys or filter values"
    };
    ManagementProblem(ProblemDetail {
        r#type: "urn:litellm:error:duplicate-query-parameter".to_owned(),
        title: "Duplicate query parameter".to_owned(),
        status: 400,
        detail: format!(
            "Repeated query parameter(s): {}. Each may appear once; use a comma-separated list for multiple {list_kind}.",
            duplicates.join(", ")
        ),
        allowed: None,
    })
}

pub async fn handle_list<Row, Output, Executor, SerializeRow>(
    spec: &ListSpec,
    executor: &Executor,
    call: ListCall<'_>,
    serialize: SerializeRow,
) -> Result<ListResponse<Output>, ManagementProblem>
where
    Executor: ListExecutor<Row> + Sync,
    Row: Send,
    SerializeRow: Fn(Row) -> Output + Send,
    Output: Serialize + Send,
{
    let plan = build_query_plan(spec, call.params, call.scope).map_err(ManagementProblem)?;
    let duplicates = duplicate_params(call.pairs);
    if !duplicates.is_empty() {
        return Err(duplicate_problem(&duplicates, false));
    }
    let total_count = executor.count(&plan.where_).await;
    let rows = executor.find_many(&plan).await;
    let total_pages = total_count.div_ceil(plan.take);
    let page = plan
        .skip
        .checked_div(plan.take)
        .and_then(|value| value.checked_add(1))
        .unwrap_or(u64::MAX);
    Ok(ListResponse {
        data: rows.into_iter().map(serialize).collect(),
        meta: ListMeta {
            total_count,
            page,
            page_size: plan.take,
            total_pages,
        },
        links: build_list_links(call.path, call.pairs, page, total_pages),
    })
}

fn facet_unknown(spec: &ListSpec, params: &BTreeMap<String, String>) -> Vec<String> {
    let mut unknown: Vec<String> = params
        .keys()
        .filter(|name| *name == SORT_PARAM || !is_known_param(spec, name))
        .cloned()
        .collect();
    unknown.sort();
    unknown
}

pub async fn handle_facet<Executor>(
    spec: &ListSpec,
    executor: &Executor,
    call: FacetCall<'_>,
) -> Result<FacetListResponse, ManagementProblem>
where
    Executor: FacetExecutor + Sync,
{
    validate_facet_params(spec, &call)?;
    let (page, page_size) = facet_pagination(spec, call.params)?;
    let where_ = facet_where(spec, call.params, call.scope).map_err(ManagementProblem)?;
    let values = executor.distinct(call.field, &where_).await;
    let (skip, size) = facet_window_start(page, page_size);
    let window: Vec<String> = values
        .into_iter()
        .skip(skip)
        .take(size.saturating_add(1))
        .collect();
    let has_more = u64::try_from(window.len()).unwrap_or(u64::MAX) > page_size;
    let data = window.into_iter().take(size).collect();
    Ok(FacetListResponse {
        data,
        meta: PageMeta {
            page,
            page_size,
            has_more,
        },
        links: build_page_links(call.path, call.pairs, page, has_more),
    })
}

fn validate_facet_params(spec: &ListSpec, call: &FacetCall<'_>) -> Result<(), ManagementProblem> {
    let unknown = facet_unknown(spec, call.params);
    if !unknown.is_empty() {
        let names: BTreeSet<&str> = unknown.iter().map(String::as_str).collect();
        return Err(ManagementProblem(unknown_query_parameter(
            &names,
            facet_allowed_params(spec),
        )));
    }
    let duplicates = duplicate_params(call.pairs);
    if duplicates.is_empty() {
        Ok(())
    } else {
        Err(duplicate_problem(&duplicates, true))
    }
}

fn facet_pagination(
    spec: &ListSpec,
    params: &BTreeMap<String, String>,
) -> Result<(u64, u64), ManagementProblem> {
    let page = parse_page(params).map_err(ManagementProblem)?;
    let page_size = parse_page_size(spec, params).map_err(ManagementProblem)?;
    Ok((page, page_size))
}

fn facet_window_start(page: u64, page_size: u64) -> (usize, usize) {
    let skip = page.saturating_sub(1).saturating_mul(page_size);
    (
        usize::try_from(skip).unwrap_or(usize::MAX),
        usize::try_from(page_size).unwrap_or(usize::MAX),
    )
}
