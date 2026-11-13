use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;

use super::AktoMirroring;
use super::error::AktoError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AktoAnswer {
    pub status: StatusCode,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailVerdict {
    pub allowed: bool,
    pub reason: String,
}

impl AktoMirroring {
    pub fn handle_guardrail_response(
        &self,
        answer: &AktoAnswer,
    ) -> Result<GuardrailVerdict, AktoError> {
        if answer.status != StatusCode::OK {
            tracing::error!(
                status = answer.status.as_u16(),
                "Akto returned an unexpected status"
            );
            return Err(AktoError::HttpStatus {
                status: answer.status,
            });
        }
        let parsed: GuardrailValue = serde_json::from_str(&answer.body).map_err(|source| {
            tracing::error!(
                bytes = answer.body.len(),
                "Akto returned a non-JSON body for status 200"
            );
            AktoError::ResponseJson { source }
        })?;
        verdict_from(&parsed)
    }
}

fn verdict_from(value: &GuardrailValue) -> Result<GuardrailVerdict, AktoError> {
    let GuardrailValue::Object(result) = value else {
        return Ok(GuardrailVerdict {
            allowed: true,
            reason: String::new(),
        });
    };
    let Some(GuardrailValue::Object(data)) = result.get("data") else {
        return Ok(GuardrailVerdict {
            allowed: true,
            reason: String::new(),
        });
    };
    let Some(GuardrailValue::Object(guardrails)) = data.get("guardrailsResult") else {
        return Ok(GuardrailVerdict {
            allowed: true,
            reason: String::new(),
        });
    };
    let allowed = match guardrails.get("Allowed") {
        None => true,
        Some(GuardrailValue::Null) => false,
        Some(GuardrailValue::Boolean(flag)) => *flag,
        Some(_) => {
            return Err(AktoError::ResponseField {
                field: "data.guardrailsResult.Allowed",
            });
        }
    };
    let reason = match guardrails.get("Reason") {
        None | Some(GuardrailValue::Null) => String::new(),
        Some(GuardrailValue::Text(text)) => text.clone(),
        Some(_) => {
            return Err(AktoError::ResponseField {
                field: "data.guardrailsResult.Reason",
            });
        }
    };
    Ok(GuardrailVerdict { allowed, reason })
}
