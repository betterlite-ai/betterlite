use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_core::routes::catalog::RouteCatalog;
use serde::Deserialize as _;
use serde::de::value::{Error as ValueError, StrDeserializer};

use crate::guardrail_message_scope::scoped_indices::{
    ScanScope, ScopeFlags, scoped_message_indices,
};
use crate::guardrail_translation::BaseTranslation;
use crate::unified_dispatch::registry::TranslationRegistry;

use super::Object;
use super::helpers::{MessageRow, as_plain_dict};

// LiteLLM's guardrail translation handlers: the structured messages one call type exposes
pub trait StructuredMessageSource {
    #[must_use]
    fn structured_messages(&self, request_data: &Object) -> Option<Vec<GuardrailValue>>;
}

impl<T: BaseTranslation> StructuredMessageSource for T {
    fn structured_messages(&self, request_data: &Object) -> Option<Vec<GuardrailValue>> {
        let messages = self.get_structured_messages(request_data)?;
        Some(
            messages
                .iter()
                .map(|message| as_plain_dict(MessageRow::Typed(message)))
                .collect(),
        )
    }
}

#[derive(Clone, Copy)]
pub struct PostCallContextRequest<'call> {
    pub request_data: &'call Object,
    pub logging_call_type: Option<&'call str>,
    pub scope: ScopeFlags,
    pub translations: &'call TranslationRegistry<&'call dyn StructuredMessageSource>,
    pub routes: &'call RouteCatalog,
}

// LiteLLM's _post_call_context: the scoped conversation and the request tools, or nothing when the call type is unknown
#[must_use]
pub fn post_call_context(
    request: PostCallContextRequest<'_>,
) -> (Vec<GuardrailValue>, Option<Vec<GuardrailValue>>) {
    let PostCallContextRequest {
        request_data,
        logging_call_type,
        scope,
        translations,
        routes,
    } = request;
    let Some(call_type) = resolve_call_type(request_data, logging_call_type, routes) else {
        return (Vec::new(), None);
    };
    let Some(translation) = translations.get(call_type).map(|entry| entry.translation) else {
        return (Vec::new(), None);
    };
    let structured = translation
        .structured_messages(request_data)
        .unwrap_or_default();
    let indices = scoped_message_indices(&structured, &scope);
    if indices.is_empty() {
        return (Vec::new(), None);
    }
    let messages = indices
        .into_iter()
        .filter_map(|index| structured.get(index))
        .map(|message| as_plain_dict(MessageRow::Plain(message)))
        .collect();
    (messages, request_tools(request_data, scope))
}

fn request_tools(request_data: &Object, scope: ScopeFlags) -> Option<Vec<GuardrailValue>> {
    if scope.scan != ScanScope::AllMessages {
        return None;
    }
    match request_data.get("tools") {
        Some(GuardrailValue::Array(tools)) if !tools.is_empty() => Some(tools.clone()),
        _ => None,
    }
}

// LiteLLM's call_type chain: the route's first call type, else the logging object's, else the request's
fn resolve_call_type(
    request_data: &Object,
    logging_call_type: Option<&str>,
    routes: &RouteCatalog,
) -> Option<CallType> {
    route_call_type(request_data, routes).or_else(|| {
        first_call_type(
            logging_call_type,
            nested_text(request_data, "litellm_logging_obj", "call_type"),
        )
    })
}

fn route_call_type(request_data: &Object, routes: &RouteCatalog) -> Option<CallType> {
    let route = nested_text(
        request_data,
        "litellm_metadata",
        "user_api_key_request_route",
    )?;
    let call_types = routes.call_types_for(route).ok().flatten()?;
    call_types.first().copied()
}

// The first non-empty name decides; an unknown name stops the chain instead of falling through
fn first_call_type(logging: Option<&str>, request_logging: Option<&str>) -> Option<CallType> {
    let name = logging
        .filter(|name| !name.is_empty())
        .or_else(|| request_logging.filter(|name| !name.is_empty()))?;
    CallType::deserialize(StrDeserializer::<ValueError>::new(name)).ok()
}

fn nested_text<'data>(data: &'data Object, outer: &str, key: &str) -> Option<&'data str> {
    let GuardrailValue::Object(fields) = data.get(outer)? else {
        return None;
    };
    match fields.get(key) {
        Some(GuardrailValue::Text(text)) => Some(text.as_str()),
        _ => None,
    }
}
