use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::fingerprint::stream_item_field;
use uuid::Uuid;

use crate::guardrails::chat_guardrail_translation::extraction::json_access::integer_value;

// The caller supplies the random id and the clock, so the identity is a pure function of its inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreshIdentity {
    pub id: Uuid,
    pub created: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamIdentity {
    pub id: String,
    pub created: i64,
    pub model: String,
}

// The first relayed chunk with a non-empty string id supplies all three fields; each of created and model falls back on its own.
#[must_use]
pub fn blocked_stream_identity(
    exception_model: &str,
    responses_so_far: &[StreamItem],
    fresh: &FreshIdentity,
) -> StreamIdentity {
    let identified = responses_so_far
        .iter()
        .find_map(|item| match stream_item_field(item, "id") {
            Some(GuardrailValue::Text(id)) if !id.is_empty() => Some((id, item)),
            _ => None,
        });
    let Some((id, source)) = identified else {
        return StreamIdentity {
            id: format!("chatcmpl-{}", fresh.id),
            created: fresh.created,
            model: exception_model.to_owned(),
        };
    };
    let model = match stream_item_field(source, "model") {
        Some(GuardrailValue::Text(model)) if !model.is_empty() => model,
        _ => exception_model,
    };
    StreamIdentity {
        id: id.clone(),
        created: integer_value(stream_item_field(source, "created")).unwrap_or(fresh.created),
        model: model.to_owned(),
    }
}
