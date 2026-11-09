use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatUsage;
use indexmap::IndexMap;

use crate::agentcore_response::error::AgentCoreReplyError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentCoreUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

impl AgentCoreUsage {
    pub fn from_value(value: &Json) -> Result<Option<Self>, AgentCoreReplyError> {
        let Json::Object(fields) = value else {
            if matches!(value, Json::Null) {
                return Ok(None);
            }
            return Err(AgentCoreReplyError::UsageNotObject);
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
    pub fn to_chat_usage(&self) -> ChatUsage {
        ChatUsage {
            prompt_tokens: self.input_tokens.unwrap_or(0),
            completion_tokens: self.output_tokens.unwrap_or(0),
            total_tokens: self.total_tokens.unwrap_or(0),
            prompt_tokens_details: None,
            completion_tokens_details: None,
        }
    }
}

fn token(
    fields: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<u64>, AgentCoreReplyError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => {
            if let Some(found) = number.as_u64() {
                return Ok(Some(found));
            }
            if number.as_i64().is_some_and(|found| found < 0) {
                return Err(AgentCoreReplyError::UsageFieldNegative { field });
            }
            Err(AgentCoreReplyError::UsageFieldNotInteger { field })
        }
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(AgentCoreReplyError::UsageFieldNotInteger { field })
        }
    }
}
