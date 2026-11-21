use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

#[derive(Debug, Clone, PartialEq)]
pub struct ModifyFailure {
    pub message: String,
    pub model: Option<String>,
    pub original_response: Option<GuardrailValue>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HookFailure {
    #[error("the guardrail blocked the response")]
    ModifyResponse(Box<ModifyFailure>),
    #[error("the guardrail refused the scan with status {status}")]
    Rejected { status: u16, detail: GuardrailValue },
    #[error("{message}")]
    Failed { message: String },
}

// What a raw Anthropic stream needs to continue as a complete blocked message.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockedMessage {
    pub message: String,
    pub model: String,
    pub original_response: GuardrailValue,
}

// What a chat stream needs to be replaced by the judge's block reply, usage and all.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockedChat {
    pub message: String,
    pub model: Option<String>,
    pub usage: Option<GuardrailValue>,
}

// LiteLLM's serialize_http_exception_detail: flat and nested error shapes name the message, and anything else stays JSON.
#[must_use]
pub fn detail_message(detail: &GuardrailValue) -> String {
    let GuardrailValue::Object(fields) = detail else {
        return match detail {
            GuardrailValue::Text(text) => text.clone(),
            other @ GuardrailValue::Null
            | other @ GuardrailValue::Boolean(_)
            | other @ GuardrailValue::SignedInteger(_)
            | other @ GuardrailValue::UnsignedInteger(_)
            | other @ GuardrailValue::Decimal(_)
            | other @ GuardrailValue::Array(_)
            | other @ GuardrailValue::Object(_) => encode(other),
        };
    };
    if let Some(GuardrailValue::Text(text)) = fields.get("error") {
        return text.clone();
    }
    if let Some(GuardrailValue::Object(error)) = fields.get("error")
        && let Some(GuardrailValue::Text(message)) = error.get("message")
    {
        return message.clone();
    }
    if let Some(GuardrailValue::Text(message)) = fields.get("message") {
        return message.clone();
    }
    encode(detail)
}

fn encode(value: &GuardrailValue) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| {
        tracing::error!("the guardrail detail could not be serialized: {error}");
        String::new()
    })
}
