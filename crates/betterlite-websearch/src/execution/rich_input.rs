use serde_json::value::RawValue;

use crate::call_transformation::wire::Input;
use crate::records::RichWebSearchInput;

const MAX_SEARCH_QUERIES: usize = 5;

#[must_use]
pub fn rich_search_input(tool_input: &Input) -> Option<RichWebSearchInput> {
    let Input::Object(fields) = tool_input else {
        return None;
    };
    let objective = fields
        .get("objective")
        .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
        .filter(|objective| !objective.trim().is_empty());
    let search_queries = fields
        .get("search_queries")
        .and_then(|value| query_list(value));
    if objective.is_none() && search_queries.is_none() {
        return None;
    }
    Some(RichWebSearchInput {
        objective,
        search_queries,
    })
}

fn query_list(value: &RawValue) -> Option<Vec<String>> {
    let items: Vec<Box<RawValue>> = serde_json::from_str(value.get()).ok()?;
    let queries: Vec<String> = items
        .iter()
        .filter_map(|item| serde_json::from_str::<String>(item.get()).ok())
        .filter(|query| !query.trim().is_empty())
        .collect();
    if queries.is_empty() {
        return None;
    }
    Some(queries.into_iter().take(MAX_SEARCH_QUERIES).collect())
}
