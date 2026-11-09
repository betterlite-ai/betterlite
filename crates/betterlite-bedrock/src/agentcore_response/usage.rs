use betterlite_models::decisions::Json;
use betterlite_openai::chat::response::ChatUsage;
use indexmap::IndexMap;

use crate::agentcore_response::error::AgentCoreReplyError;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentCoreUsage {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
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

    pub fn to_chat_usage(&self) -> Result<ChatUsage, AgentCoreReplyError> {
        Ok(ChatUsage {
            prompt_tokens: count(self.input_tokens, "inputTokens")?,
            completion_tokens: count(self.output_tokens, "outputTokens")?,
            total_tokens: count(self.total_tokens, "totalTokens")?,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        })
    }
}

fn token(
    fields: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<i64>, AgentCoreReplyError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => {
            let Some(found) = number.as_i64() else {
                return Err(AgentCoreReplyError::UsageFieldNotInteger { field });
            };
            Ok(Some(found))
        }
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(AgentCoreReplyError::UsageFieldNotInteger { field })
        }
    }
}

fn count(value: Option<i64>, field: &'static str) -> Result<u64, AgentCoreReplyError> {
    let Some(found) = value else {
        return Ok(0);
    };
    let Ok(converted) = u64::try_from(found) else {
        return Err(AgentCoreReplyError::UsageFieldNegative { field });
    };
    Ok(converted)
}
