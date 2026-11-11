use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use betterlite_prompts::response_tool_calls::structured::{
    MappedHandler, RouteCallTypes, resolve_structured_messages,
};

use crate::straiker_webhook::jsonable::record_dict;

#[must_use]
pub fn request_structured_messages<Key>(
    request_data: &Object,
    routes: &dyn RouteCallTypes<Key>,
    mappings: &[MappedHandler<'_, Key>],
) -> Option<Vec<Object>>
where
    Key: Eq,
{
    if let Some(Json::List(items)) = request_data.get("messages") {
        return Some(items.iter().filter_map(Json::as_object).cloned().collect());
    }
    let route = request_data
        .get("litellm_metadata")
        .and_then(Json::as_object)
        .and_then(|metadata| metadata.get("user_api_key_request_route"))
        .and_then(Json::as_str)?;
    if route.is_empty() || !route_has_translation(route, routes, mappings) {
        return None;
    }
    let messages = resolve_structured_messages(None, request_data, mappings, routes)?;
    Some(messages.iter().filter_map(record_dict::<Message>).collect())
}

fn route_has_translation<Key>(
    route: &str,
    routes: &dyn RouteCallTypes<Key>,
    mappings: &[MappedHandler<'_, Key>],
) -> bool
where
    Key: Eq,
{
    let Some(call_types) = routes.call_types_for_route(route) else {
        return false;
    };
    call_types.iter().any(|call_type| {
        mappings
            .iter()
            .any(|mapped| mapped.call_types.contains(call_type))
    })
}

mod tests;
