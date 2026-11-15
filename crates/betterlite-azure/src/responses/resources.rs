use betterlite_http::url_path::encode_segment;
use betterlite_http::user_url::split::urlparse;
use betterlite_openai::response_resources::request::{
    ListInputItemsQuery, ResourceOperation, ResourceRequest, list_input_items_request,
};
use betterlite_prompts::arguments::json::Object;

use super::error::ResponsesError;

// urllib's parts, so the id lands before "?query" and ";params" stays on the last path segment
pub fn url_for_response_id_in_path(
    api_base: &str,
    response_id: &str,
    suffix: &str,
) -> Result<String, ResponsesError> {
    let encoded = encode_segment(response_id, "response_id")
        .map_err(|source| ResponsesError::ResponseId { source })?;
    let mut parsed = urlparse(api_base, "")?;
    parsed.path = format!("{}/{encoded}{suffix}", parsed.path.trim_end_matches('/'));
    Ok(parsed.unparse())
}

pub fn resource_request(
    api_base: &str,
    response_id: &str,
    operation: ResourceOperation,
) -> Result<ResourceRequest, ResponsesError> {
    let suffix = match operation {
        ResourceOperation::Delete | ResourceOperation::Get => "",
        ResourceOperation::Cancel => "/cancel",
    };
    let url = url_for_response_id_in_path(api_base, response_id, suffix)?;
    Ok(ResourceRequest {
        url,
        payload: Object::new(),
    })
}

// The shared builder owns the query rules (include joins, limit and order defaults); Azure only moves the id
pub fn list_input_items(
    api_base: &str,
    response_id: &str,
    query: &ListInputItemsQuery,
) -> Result<ResourceRequest, ResponsesError> {
    let mut request = list_input_items_request(response_id, api_base, query)?;
    request.url = url_for_response_id_in_path(api_base, response_id, "/input_items")?;
    Ok(request)
}
