use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

use super::LITELLM_VERSION;
use super::error::PillarError;
use super::model_provider::{ModelProviderResolver, extract_model_and_provider};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProtectMetadata {
    pub source: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProtectPayload {
    pub messages: GuardrailValue,
    pub tools: GuardrailValue,
    pub metadata: ProtectMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub model: String,
    pub provider: String,
}

fn collection_or_empty(
    data: &BTreeMap<String, GuardrailValue>,
    field: &'static str,
) -> Result<GuardrailValue, PillarError> {
    match data.get(field) {
        None | Some(GuardrailValue::Null) => Ok(GuardrailValue::Array(Vec::new())),
        Some(value @ GuardrailValue::Array(_)) => Ok(value.clone()),
        Some(_) => Err(PillarError::Field {
            field,
            expected: "a list",
        }),
    }
}

fn optional_text(
    data: &BTreeMap<String, GuardrailValue>,
    field: &'static str,
) -> Result<Option<String>, PillarError> {
    match data.get(field) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(PillarError::Field {
            field,
            expected: "a string",
        }),
    }
}

fn session_id(data: &BTreeMap<String, GuardrailValue>) -> Result<Option<String>, PillarError> {
    let metadata = match data.get("metadata") {
        None | Some(GuardrailValue::Null) => return Ok(None),
        Some(GuardrailValue::Object(fields)) => fields,
        Some(_) => {
            return Err(PillarError::Field {
                field: "metadata",
                expected: "an object",
            });
        }
    };
    match metadata.get("pillar_session_id") {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(PillarError::Field {
            field: "pillar_session_id",
            expected: "a string",
        }),
    }
}

pub fn prepare_payload(
    data: &BTreeMap<String, GuardrailValue>,
    resolver: &dyn ModelProviderResolver,
) -> Result<ProtectPayload, PillarError> {
    let messages = collection_or_empty(data, "messages")?;
    let tools = collection_or_empty(data, "tools")?;
    let user_id = optional_text(data, "user")?;
    let session_id = session_id(data)?;
    let (model, provider) = extract_model_and_provider(data, resolver)?;
    Ok(ProtectPayload {
        messages,
        tools,
        metadata: ProtectMetadata {
            source: "litellm",
            version: LITELLM_VERSION,
        },
        user_id,
        session_id,
        model,
        provider,
    })
}
