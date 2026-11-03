use super::links::{build_list_links, build_page_links};
use super::plan::build::{build_query_plan, facet_where};
use super::plan::types::{FacetExecutor, ListExecutor, ListSpec, SORT_PARAM, Scope};
use super::problem::ManagementProblem;
use crate::management_envelopes::{FacetListResponse, ListMeta, ListResponse, PageMeta};
use serde::Serialize;
use std::collections::BTreeMap;

pub async fn handle_list<Row, Output, Executor, SerializeRow>(
    spec: &ListSpec,
    executor: &Executor,
    params: &BTreeMap<String, String>,
    scope: &Scope,
    path: &str,
    pairs: &[(String, String)],
    serialize: SerializeRow,
) -> Result<ListResponse<Output>, ManagementProblem>
where
    Executor: ListExecutor<Row>,
    SerializeRow: Fn(Row) -> Output,
    Output: Serialize,
{
    let plan = build_query_plan(spec, params, scope).map_err(ManagementProblem)?;
    let total_count = executor.count(&plan.where_).await;
    let rows = executor.find_many(&plan).await;
    let total_pages = total_count.div_ceil(plan.take);
    let page = plan.skip / plan.take + 1;
    Ok(ListResponse {
        data: rows.into_iter().map(serialize).collect(),
        meta: ListMeta {
            total_count,
            page,
            page_size: plan.take,
            total_pages,
        },
        links: build_list_links(path, pairs, page, total_pages),
    })
}

pub async fn handle_facet<Executor>(
    spec: &ListSpec,
    executor: &Executor,
    params: &BTreeMap<String, String>,
    scope: &Scope,
    field: &str,
    path: &str,
    pairs: &[(String, String)],
) -> Result<FacetListResponse, ManagementProblem>
where
    Executor: FacetExecutor,
{
    if params.contains_key(SORT_PARAM) {
        return Err(ManagementProblem(super::problem::unknown_query_parameter(
            &std::collections::BTreeSet::from([SORT_PARAM]),
            super::plan::params::facet_allowed_params(spec),
        )));
    }
    let page = params
        .get("page")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1);
    let page_size = params
        .get("page_size")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(spec.default_page_size)
        .min(spec.max_page_size);
    let where_ = facet_where(spec, params, scope).map_err(ManagementProblem)?;
    let values = executor.distinct(field, &where_).await;
    let skip = page.saturating_sub(1).saturating_mul(page_size);
    let start = usize::try_from(skip).unwrap_or(usize::MAX);
    let size = usize::try_from(page_size).unwrap_or(usize::MAX);
    let window: Vec<String> = values
        .into_iter()
        .skip(start)
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
        links: build_page_links(path, pairs, page, has_more),
    })
}
