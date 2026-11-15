use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_core::routes::catalog::RouteCatalog;

use crate::unified_dispatch::stream_items::CHUNK_OBJECT;

pub struct TranslationEntry<T> {
    pub call_type: CallType,
    pub translation: T,
    pub chat_completions: bool,
}

pub struct TranslationRegistry<T> {
    pub entries: Vec<TranslationEntry<T>>,
}

pub const A2A_CALL_TYPES: [CallType; 2] = [CallType::AsendMessage, CallType::SendMessage];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseKind {
    ChatCompletion,
    Embedding,
    Transcription,
    Speech,
    Rerank,
    Image,
    TextCompletion,
    SendMessage,
    Other,
}

impl ResponseKind {
    #[must_use]
    pub fn of_stream_item(item: &GuardrailValue) -> Self {
        let GuardrailValue::Object(fields) = item else {
            return Self::Other;
        };
        match fields.get("object") {
            Some(GuardrailValue::Text(kind)) if kind == CHUNK_OBJECT => Self::ChatCompletion,
            _ => Self::Other,
        }
    }
}

#[must_use]
pub const fn infer_call_type(kind: ResponseKind) -> Option<CallType> {
    match kind {
        ResponseKind::ChatCompletion => Some(CallType::Completion),
        ResponseKind::Embedding => Some(CallType::Embedding),
        ResponseKind::Transcription => Some(CallType::Transcription),
        ResponseKind::Speech => Some(CallType::Speech),
        ResponseKind::Rerank => Some(CallType::Rerank),
        ResponseKind::Image => Some(CallType::ImageGeneration),
        ResponseKind::TextCompletion => Some(CallType::TextCompletion),
        ResponseKind::SendMessage => Some(CallType::SendMessage),
        ResponseKind::Other => None,
    }
}

impl<T> TranslationRegistry<T> {
    // a later entry for one call type replaces an earlier one, like dict.update
    #[must_use]
    pub fn get(&self, call_type: CallType) -> Option<&TranslationEntry<T>> {
        let mut found = None;
        for entry in &self.entries {
            if entry.call_type == call_type {
                found = Some(entry);
            }
        }
        found
    }
}

fn first_route_call_type(routes: &RouteCatalog, request_route: Option<&str>) -> Option<CallType> {
    let route = request_route.filter(|route| !route.is_empty())?;
    let call_types = routes.call_types_for(route).ok().flatten()?;
    call_types.first().copied()
}

#[must_use]
pub fn resolve_endpoint_translation<'registry, T>(
    registry: &'registry TranslationRegistry<T>,
    routes: &RouteCatalog,
    request_route: Option<&str>,
    first_item: Option<&GuardrailValue>,
) -> Option<(CallType, &'registry T)> {
    let call_type = first_route_call_type(routes, request_route).or_else(|| {
        first_item
            .map(ResponseKind::of_stream_item)
            .and_then(infer_call_type)
    })?;
    let entry = registry.get(call_type)?;
    Some((call_type, &entry.translation))
}

#[must_use]
pub fn resolve_transform_call_type<T>(
    registry: &TranslationRegistry<T>,
    routes: &RouteCatalog,
    request_route: Option<&str>,
) -> Option<CallType> {
    let call_type = first_route_call_type(routes, request_route)?;
    let entry = registry.get(call_type)?;
    entry.chat_completions.then_some(call_type)
}

pub fn ensure_request_metadata(
    data: &mut BTreeMap<String, GuardrailValue>,
    user_metadata: BTreeMap<String, GuardrailValue>,
) {
    if data.contains_key("litellm_metadata") || user_metadata.is_empty() {
        return;
    }
    data.insert(
        "litellm_metadata".to_owned(),
        GuardrailValue::Object(user_metadata),
    );
}
