use crate::arguments::json::{Json, Object};

const TOOL_USE_ID: &str = "tool_use_id";

#[must_use]
pub fn find_server_tool_result<'results>(
    tool_id: &str,
    web_search_results: Option<&'results [Json]>,
    tool_results: Option<&'results [Json]>,
) -> Option<&'results Object> {
    let web_results = web_search_results.unwrap_or_default();
    let tool_matches = tool_results.unwrap_or_default();
    web_results
        .iter()
        .chain(tool_matches)
        .find_map(|candidate| {
            let object = candidate.as_object()?;
            (object.get(TOOL_USE_ID).and_then(Json::as_str) == Some(tool_id)).then_some(object)
        })
}
