use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatUsage;
use indexmap::IndexMap;
use serde_json::Number;

use crate::agentcore_response::error::AgentCoreResponseError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentCoreUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

impl AgentCoreUsage {
    pub fn from_value(value: &Json) -> Result<Option<Self>, AgentCoreResponseError> {
        let Json::Object(fields) = value else {
            if matches!(value, Json::Null) {
                return Ok(None);
            }
            return Err(AgentCoreResponseError::UsageNotObject);
        };
        if fields.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self {
            input_tokens: token(fields, "inputTokens")?,
            output_tokens: token(fields, "outputTokens")?,
            total_tokens: token(fields, "totalTokens")?,
        }))
    }

    #[must_use]
    pub const fn to_chat_usage(&self) -> ChatUsage {
        ChatUsage {
            prompt_tokens: self.input_tokens,
            completion_tokens: self.output_tokens,
            total_tokens: self.total_tokens,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        }
    }
}

fn token(
    fields: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<u64, AgentCoreResponseError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number_token(number, field),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(AgentCoreResponseError::UsageFieldNotInteger { field })
        }
    }
}

fn number_token(number: &Number, field: &'static str) -> Result<u64, AgentCoreResponseError> {
    if let Some(found) = number.as_u64() {
        return Ok(found);
    }
    let Some(found) = number.as_i64() else {
        return Err(AgentCoreResponseError::UsageFieldNotInteger { field });
    };
    let Ok(positive) = u64::try_from(found) else {
        return Err(AgentCoreResponseError::UsageFieldNegative { field });
    };
    Ok(positive)
}
