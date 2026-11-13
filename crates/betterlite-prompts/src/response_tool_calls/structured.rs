use crate::arguments::json::{Json, Object};
use crate::message_content::Message;

// The per-surface translation handlers and the route table belong to their own modules, so the resolution reads both through this policy.
pub trait StructuredMessageHandler {
    fn get_structured_messages(&self, request_kwargs: &Object) -> Option<Vec<Message>>;
}

pub trait RouteCallTypes<Key> {
    fn call_types_for_route(&self, route: &str) -> Option<Vec<Key>>;
}

pub struct MappedHandler<'mapped, Key> {
    pub call_types: &'mapped [Key],
    pub handler: &'mapped dyn StructuredMessageHandler,
}

// A request that already carries chat messages keeps them; otherwise the route picks one handler, or every handler is tried in mapping order.
#[must_use]
pub fn resolve_structured_messages<Key>(
    messages: Option<&[Message]>,
    request_kwargs: &Object,
    mappings: &[MappedHandler<'_, Key>],
    routes: &dyn RouteCallTypes<Key>,
) -> Option<Vec<Message>>
where
    Key: Eq,
{
    if let Some(given) = messages {
        return Some(given.to_vec());
    }
    let request_route = request_kwargs
        .get("litellm_metadata")
        .and_then(Json::as_object)
        .and_then(|metadata| metadata.get("user_api_key_request_route"))
        .and_then(Json::as_str);
    let selected = request_route
        .and_then(|route| routes.call_types_for_route(route))
        .and_then(|call_types| first_mapped_index(mappings, &call_types));
    let Some(mapped) = selected.and_then(|index| mappings.get(index)) else {
        return mappings
            .iter()
            .find_map(|mapped| mapped.handler.get_structured_messages(request_kwargs));
    };
    mapped.handler.get_structured_messages(request_kwargs)
}

fn first_mapped_index<Key>(mappings: &[MappedHandler<'_, Key>], call_types: &[Key]) -> Option<usize>
where
    Key: Eq,
{
    call_types.iter().find_map(|call_type| {
        mappings
            .iter()
            .position(|mapped| mapped.call_types.contains(call_type))
    })
}

mod tests;
