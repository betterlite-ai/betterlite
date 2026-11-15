use betterlite_openai::chat_choices::choices::transform_choices;
use betterlite_openai::chat_choices::wire::{ReplyChoice, TransformedChoice};
use betterlite_openai::openai_base_config::Object;
use serde::Deserialize;
use serde::Serialize;
use serde::de::value::MapDeserializer;
use uuid::Uuid;

use crate::partner_openai::error::PartnerOpenAiError;

// The fields transform_response reads off the provider's OpenAI-shaped completion.
#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    model: Option<String>,
    choices: Vec<ReplyChoice>,
    #[serde(default)]
    usage: Option<Object>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LlamaReply {
    pub id: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<TransformedChoice>,
    // The provider's usage object travels unchanged; the shared reply builder owns its conversion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Object>,
}

// LiteLLM defaults model, id and created, then hands the choices to _transform_choices.
pub fn decode_reply(
    reply: &Object,
    requested_model: &str,
    json_mode: bool,
    identifier: fn() -> Uuid,
) -> Result<LlamaReply, PartnerOpenAiError> {
    let deserializer = MapDeserializer::<_, serde_json::Error>::new(
        reply.iter().map(|(key, value)| (key.as_str(), value)),
    );
    let envelope = Envelope::deserialize(deserializer)
        .map_err(|source| PartnerOpenAiError::Reply { source })?;
    let choices = transform_choices(&envelope.choices, json_mode, None, identifier)?;
    Ok(LlamaReply {
        id: envelope.id.unwrap_or_default(),
        created: envelope.created.unwrap_or(0),
        model: envelope.model.unwrap_or_else(|| requested_model.to_owned()),
        choices,
        usage: envelope.usage,
    })
}
