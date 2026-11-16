use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::error::Category;
use thiserror::Error;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::agent_365_tokens::DEFENDER_STATUS_EVALUATED;

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
    NonObjectBody {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Agent 365 endpoint returned a verdict without a boolean 'allowed' field")]
    MissingAllowed,
}

#[derive(Debug, Deserialize)]
struct EvaluateWire {
    #[serde(default)]
    allowed: AllowedField,
    #[serde(default)]
    defender: DefenderField,
    #[serde(rename = "correlationId", default)]
    correlation_id: TextField,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum AllowedField {
    Boolean(bool),
    Other(IgnoredAny),
    #[default]
    Missing,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum DefenderField {
    Object(DefenderWire),
    Other(IgnoredAny),
    #[default]
    Missing,
}

#[derive(Debug, Default, Deserialize)]
struct DefenderWire {
    status: Option<TextField>,
    verdict: Option<TextField>,
    message: Option<TextField>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum TextField {
    Text(String),
    Other(IgnoredAny),
    #[default]
    Missing,
}

pub fn decode_evaluate_response(body: &[u8]) -> Result<EvaluateVerdict, EvaluateResponseError> {
    let wire: EvaluateWire = serde_json::from_slice(body).map_err(|source| {
        // serde reports well-formed JSON of the wrong shape as a data error and broken JSON as a syntax error
        if source.classify() == Category::Data {
            EvaluateResponseError::NonObjectBody { source }
        } else {
            EvaluateResponseError::NonJsonBody { source }
        }
    })?;
    let AllowedField::Boolean(allowed) = wire.allowed else {
        return Err(EvaluateResponseError::MissingAllowed);
    };
    let defender = match wire.defender {
        DefenderField::Object(defender) => DefenderResult {
            status: defender.status.and_then(TextField::into_text),
            verdict: defender.verdict.and_then(TextField::into_text),
            message: defender.message.and_then(TextField::into_text),
        },
        // LiteLLM reads a defender that is not an object as an empty one
        DefenderField::Other(_) | DefenderField::Missing => DefenderResult::default(),
    };
    Ok(EvaluateVerdict {
        allowed,
        defender,
        correlation_id: wire.correlation_id.into_text(),
    })
}

impl TextField {
    fn into_text(self) -> Option<String> {
        match self {
            Self::Text(text) => Some(text),
            Self::Other(_) | Self::Missing => None,
        }
    }
}

impl EvaluateVerdict {
    #[must_use]
    pub fn defender_evaluated(&self) -> bool {
        let status = self.defender.status.as_deref();
        status == Some(DEFENDER_STATUS_EVALUATED)
    }

    // LiteLLM's f"Microsoft Defender did not evaluate the call (defender.status={defender_status or 'missing'})"
    #[must_use]
    pub fn unevaluated_reason(&self) -> String {
        let status = self.defender.status.as_deref().unwrap_or("missing");
        format!("Microsoft Defender did not evaluate the call (defender.status={status})")
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
        let mut detail = IndexMap::new();
        detail.insert("error".to_owned(), GuardrailValue::Text(self.error.clone()));
        detail.insert(
            "message".to_owned(),
            GuardrailValue::Text(self.message.clone()),
        );
        detail.insert("tool".to_owned(), GuardrailValue::Text(self.tool.clone()));
        detail
    }
}

impl BlockedDetail {
    #[must_use]
    pub fn detail(&self) -> IndexMap<String, GuardrailValue> {
        let mut detail = IndexMap::new();
        detail.insert("error".to_owned(), GuardrailValue::Text(self.error.clone()));
        detail.insert(
            "message".to_owned(),
            GuardrailValue::Text(self.message.clone()),
        );
        detail.insert("tool".to_owned(), GuardrailValue::Text(self.tool.clone()));
        let correlation = match &self.correlation_id {
            Some(correlation_id) => GuardrailValue::Text(correlation_id.clone()),
            None => GuardrailValue::Null,
        };
        detail.insert("correlation_id".to_owned(), correlation);
        detail
    }
}

mod tests;
