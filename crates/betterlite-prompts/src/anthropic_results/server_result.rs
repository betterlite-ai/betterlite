use crate::arguments::json::{Json, Object};

const TOOL_USE_ID: &str = "tool_use_id";

#[must_use]
pub fn find_server_tool_result<'results>(
    tool_id: &str,
    web_search_results: &'results [Json],
    tool_results: &'results [Json],
) -> Option<&'results Object> {
    web_search_results
        .iter()
        .chain(tool_results)
        .find_map(|candidate| {
            let object = candidate.as_object()?;
            (object.get(TOOL_USE_ID).and_then(Json::as_str) == Some(tool_id)).then_some(object)
        })
}
