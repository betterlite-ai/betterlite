use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

use crate::straiker::delivery::settings::ApiVersion;
use crate::straiker::v3::response_mapping::{
    StraikerWebhookAction, StraikerWebhookResponse, response,
};
use crate::straiker_webhook::failure::WebhookFailure;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
pub enum VerdictAction {
    #[serde(rename = "NONE")]
    #[default]
    None,
    #[serde(rename = "BLOCKED")]
    Blocked,
    #[serde(rename = "GUARDRAIL_INTERVENED")]
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

// LiteLLM's StraikerWebhookResponse pydantic model: the v1 webhook's answer, and the read shape both readers return.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Verdict {
    #[serde(default)]
    pub action: VerdictAction,
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    pub texts: Option<Vec<String>>,
    pub schema_version: Option<String>,
    #[serde(rename = "turnId")]
    pub turn_id: Option<String>,
}

pub fn parse(api_version: ApiVersion, body: &str) -> Result<Verdict, WebhookFailure> {
    let fields = json_object(body)?;
    if api_version == ApiVersion::V1 {
        return serde_json::from_str(body)
            .map_err(|source| schema_failure(format!("invalid response schema: {source}")));
    }
    let parsed = response(&fields).ok_or_else(|| {
        schema_failure("invalid response schema: no allow or block decision".to_owned())
    })?;
    Ok(v3_verdict(parsed))
}

fn v3_verdict(parsed: StraikerWebhookResponse) -> Verdict {
    Verdict {
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

const fn schema_failure(message: String) -> WebhookFailure {
    WebhookFailure {
        message,
        is_unreachable: false,
        retryable: false,
    }
}
