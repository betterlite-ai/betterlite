use betterlite_models::decisions::Json;

use super::types::SuccessResult;

pub const MCP_TOOL_CALL_TYPE: &str = "call_mcp_tool";

const VECTOR_STORE_SEARCH_OBJECT: &str = "vector_store.search_results.page";
const SEARCH_OBJECT: &str = "search";

#[must_use]
pub fn is_recognized_call_type(result: &SuccessResult, call_type: &str) -> bool {
    if call_type == MCP_TOOL_CALL_TYPE {
        return true;
    }
    match result {
        SuccessResult::Chat(_) | SuccessResult::Embeddings(_) | SuccessResult::Chunk(_) => true,
        SuccessResult::Other(Json::Object(fields)) => matches!(
            fields.get("object"),
            Some(Json::Text(object))
                if object == VECTOR_STORE_SEARCH_OBJECT || object == SEARCH_OBJECT
        ),
        SuccessResult::Other(
            Json::List(_) | Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_),
        ) => false,
    }
}

#[must_use]
pub fn is_interactions_create(call_type: &str) -> bool {
    matches!(
        call_type,
        "create_interaction" | "acreate_interaction" | "create" | "acreate"
    )
}
