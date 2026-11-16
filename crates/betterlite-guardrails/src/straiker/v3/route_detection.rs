use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::response_tool_calls::structured::RouteCallTypes;

use crate::straiker_webhook::metadata::merged_metadata;

// Whether the route the proxy stamped on the request serves this call type.
#[must_use]
pub fn route_is(
    request_data: &Object,
    call_type: CallType,
    routes: &dyn RouteCallTypes<CallType>,
) -> bool {
    let metadata = merged_metadata(request_data);
    let Some(route) = metadata
        .get("user_api_key_request_route")
        .and_then(Json::as_str)
        .filter(|text| !text.is_empty())
    else {
        return false;
    };
    routes
        .call_types_for_route(route)
        .is_some_and(|call_types| call_types.contains(&call_type))
}

mod tests;
