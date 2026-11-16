use indexmap::IndexMap;
use thiserror::Error;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::agent_365_tokens::DEFENDER_STATUS_EVALUATED;
use crate::primitives::json_value::JsonValue;

// LiteLLM's _DefenderResult; a missing field is the same as an absent one
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefenderResult {
    pub status: Option<String>,
    pub verdict: Option<String>,
    pub message: Option<String>,
}

// LiteLLM's _EvaluateResponse after the shape checks in _enforce_verdict
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluateVerdict {
    pub allowed: bool,
    pub defender: DefenderResult,
    pub correlation_id: Option<String>,
}

// LiteLLM's _UnavailableDetail, also the detail of a rejected evaluation request
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnavailableDetail {
    pub error: String,
    pub message: String,
    pub tool: String,
}

// LiteLLM's _BlockedDetail
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedDetail {
    pub error: String,
    pub message: String,
    pub tool: String,
    pub correlation_id: Option<String>,
}

// The verdict names LiteLLM writes into the standard logging record
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Block,
    Rejected,
    Throttled,
    Unscanned,
    Unavailable,
}

#[derive(Debug, Error)]
pub enum EvaluateResponseError {
    #[error("the Agent 365 endpoint returned a non-JSON body")]
    NonJsonBody {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Agent 365 endpoint returned a non-object JSON body")]
    NonObjectBody,
    #[error("the Agent 365 endpoint returned a verdict without a boolean 'allowed' field")]
    MissingAllowed,
}

// The body is read as a JSON tree first, so an array never fills the verdict's fields by position
pub fn decode_evaluate_response(body: &[u8]) -> Result<EvaluateVerdict, EvaluateResponseError> {
    let parsed: JsonValue = serde_json::from_slice(body)
        .map_err(|source| EvaluateResponseError::NonJsonBody { source })?;
    let JsonValue::Object(verdict) = parsed else {
        return Err(EvaluateResponseError::NonObjectBody);
    };
    let Some(&JsonValue::Bool(allowed)) = verdict.get("allowed") else {
        return Err(EvaluateResponseError::MissingAllowed);
    };
    let defender = match verdict.get("defender") {
        Some(JsonValue::Object(defender)) => DefenderResult {
            status: text_of(defender.get("status")),
            verdict: text_of(defender.get("verdict")),
            message: text_of(defender.get("message")),
        },
        // LiteLLM reads a defender that is not an object as an empty one
        _ => DefenderResult::default(),
    };
    Ok(EvaluateVerdict {
        allowed,
        defender,
        correlation_id: text_of(verdict.get("correlationId")),
    })
}

// LiteLLM's isinstance(value, str): a value of any other type reads as absent
fn text_of(value: Option<&JsonValue>) -> Option<String> {
    match value {
        Some(JsonValue::String(text)) => Some(text.clone()),
        _ => None,
    }
}

impl EvaluateVerdict {
    #[must_use]
    pub fn defender_evaluated(&self) -> bool {
        match self.defender.status.as_deref() {
            Some(status) => status == DEFENDER_STATUS_EVALUATED,
            None => false,
        }
    }

    // LiteLLM's f"Microsoft Defender did not evaluate the call (defender.status={defender_status or 'missing'})"
    #[must_use]
    pub fn unevaluated_reason(&self) -> String {
        format!(
            "Microsoft Defender did not evaluate the call (defender.status={})",
            self.defender.status.as_deref().unwrap_or("missing")
        )
    }

    #[must_use]
    pub fn blocked_message(&self, tool_name: &str) -> String {
        match self.defender.message.as_deref() {
            Some(message) => message.to_owned(),
            None => format!(
                "Invocation of '{tool_name}' is blocked by Microsoft Threat Detection policies configured by your administrator."
            ),
        }
    }
}

impl Verdict {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Allow => "Allow",
            Self::Block => "Block",
            Self::Rejected => "Rejected",
            Self::Throttled => "Throttled",
            Self::Unscanned => "Unscanned",
            Self::Unavailable => "Unavailable",
        }
    }
}

impl UnavailableDetail {
    #[must_use]
    pub fn detail(&self) -> IndexMap<String, GuardrailValue> {
        IndexMap::from([
            ("error".to_owned(), GuardrailValue::Text(self.error.clone())),
            (
                "message".to_owned(),
                GuardrailValue::Text(self.message.clone()),
            ),
            ("tool".to_owned(), GuardrailValue::Text(self.tool.clone())),
        ])
    }
}

impl From<&BlockedDetail> for UnavailableDetail {
    fn from(detail: &BlockedDetail) -> Self {
        Self {
            error: detail.error.clone(),
            message: detail.message.clone(),
            tool: detail.tool.clone(),
        }
    }
}

fn correlation_value(correlation_id: Option<&str>) -> GuardrailValue {
    match correlation_id {
        Some(correlation_id) => GuardrailValue::Text(correlation_id.to_owned()),
        None => GuardrailValue::Null,
    }
}

impl BlockedDetail {
    #[must_use]
    pub fn detail(&self) -> IndexMap<String, GuardrailValue> {
        let mut detail = UnavailableDetail::from(self).detail();
        let correlation = correlation_value(self.correlation_id.as_deref());
        detail.insert("correlation_id".to_owned(), correlation);
        detail
    }
}

mod tests;
