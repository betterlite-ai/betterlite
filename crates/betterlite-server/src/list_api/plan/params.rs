use crate::list_api::plan::types::{
    ComparisonOp, FilterOp, ListSpec, PAGE_PARAM, PAGE_SIZE_PARAM, SEARCH_PARAM, SORT_PARAM,
};

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
