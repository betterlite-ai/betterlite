use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

use crate::generic_guardrail_contracts::GuardrailResponseAction;
use crate::straiker::delivery::settings::ApiVersion;
use crate::straiker::v3::response_mapping::{StraikerWebhookAction, response};
use crate::straiker_webhook::failure::WebhookFailure;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerdictAction {
    None,
    Blocked,
    Intervened,
}

impl VerdictAction {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Blocked => "BLOCKED",
            Self::Intervened => "GUARDRAIL_INTERVENED",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Verdict {
    pub action: VerdictAction,
    pub blocked_reason: Option<String>,
    pub blocked_by: Vec<String>,
    pub texts: Option<Vec<String>>,
    pub schema_version: Option<String>,
    pub turn_id: Option<String>,
}

// LiteLLM's StraikerWebhookResponse pydantic model: the v1 webhook's answer.
#[derive(Debug, Deserialize)]
struct V1Response {
    #[serde(default)]
    action: GuardrailResponseAction,
    #[serde(default)]
    blocked_reason: Option<String>,
    #[serde(default)]
    blocked_by: Vec<String>,
    #[serde(default)]
    texts: Option<Vec<String>>,
    #[serde(default)]
    schema_version: Option<String>,
    #[serde(default, rename = "turnId")]
    turn_id: Option<String>,
}

pub fn parse(api_version: ApiVersion, body: &str) -> Result<Verdict, WebhookFailure> {
    match api_version {
        ApiVersion::V3 => parse_v3(body),
        ApiVersion::V1 => parse_v1(body),
    }
}

pub fn parse_v1(body: &str) -> Result<Verdict, WebhookFailure> {
    json_object(body)?;
    let parsed: V1Response = serde_json::from_str(body)
        .map_err(|source| schema_failure(format!("invalid response schema: {source}")))?;
    Ok(Verdict {
        action: action_of(parsed.action),
        blocked_reason: parsed.blocked_reason,
        blocked_by: parsed.blocked_by,
        texts: parsed.texts,
        schema_version: parsed.schema_version,
        turn_id: parsed.turn_id,
    })
}

pub fn parse_v3(body: &str) -> Result<Verdict, WebhookFailure> {
    let fields = json_object(body)?;
    let parsed = response(&fields).ok_or_else(|| {
        schema_failure("invalid response schema: no allow or block decision".to_owned())
    })?;
    Ok(Verdict {
        action: match parsed.action {
            StraikerWebhookAction::Blocked => VerdictAction::Blocked,
            StraikerWebhookAction::NoAction => VerdictAction::None,
            StraikerWebhookAction::GuardrailIntervened => VerdictAction::Intervened,
        },
        blocked_reason: parsed.blocked_reason,
        blocked_by: parsed.blocked_by,
        texts: None,
        schema_version: None,
        turn_id: parsed.turn_id,
    })
}

fn action_of(action: GuardrailResponseAction) -> VerdictAction {
    match action {
        GuardrailResponseAction::Blocked => VerdictAction::Blocked,
        GuardrailResponseAction::None => VerdictAction::None,
        GuardrailResponseAction::GuardrailIntervened => VerdictAction::Intervened,
    }
}

fn json_object(body: &str) -> Result<Object, WebhookFailure> {
    let value: Json = serde_json::from_str(body)
        .map_err(|source| schema_failure(format!("invalid response schema: {source}")))?;
    match value {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(schema_failure(format!(
                "invalid response schema: expected an object, got {}",
                json_kind(&value)
            )))
        }
    }
}

fn json_kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) if number.is_f64() => "float",
        Json::Number(_) => "int",
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}

fn schema_failure(message: String) -> WebhookFailure {
    WebhookFailure {
        message,
        is_unreachable: false,
        retryable: false,
    }
}
