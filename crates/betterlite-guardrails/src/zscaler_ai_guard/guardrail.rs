use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_http::proxy::limits::{ReadError, read_bounded};
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;

use super::error::ZscalerAiGuardError;
use super::printed::{quote_text, repr_text, str_text};
use super::wire::{Direction, PolicyRequest, PolicyResponse};
use super::{Inputs, MAX_RESPONSE_BYTES, ZscalerAiGuard, metadata, settings};
use crate::generic_guardrail_contracts::GuardrailInputType;

struct UserKeyHeaders {
    alias: Option<String>,
    user_id: Option<String>,
    team_id: Option<String>,
}

impl ZscalerAiGuard {
    pub async fn apply_guardrail(
        &self,
        inputs: Inputs,
        request_data: &BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
    ) -> Result<Inputs, ZscalerAiGuardError> {
        let policy_id = metadata::selected_policy_id(request_data, self.settings.policy_id)?;
        let user = UserKeyHeaders {
            alias: if self.settings.send.alias {
                Some(metadata::header_value(request_data, "user_api_key_alias")?)
            } else {
                None
            },
            user_id: if self.settings.send.user_id {
                Some(metadata::header_value(
                    request_data,
                    "user_api_key_user_id",
                )?)
            } else {
                None
            },
            team_id: if self.settings.send.team_id {
                Some(metadata::header_value(
                    request_data,
                    "user_api_key_team_id",
                )?)
            } else {
                None
            },
        };
        if inputs.texts.is_empty() {
            return Ok(inputs);
        }
        let content = inputs.texts.join(" ");
        let direction = match input_type {
            GuardrailInputType::Request => Direction::In,
            GuardrailInputType::Response => Direction::Out,
        };
        self.send(direction, &content, &policy_id, &user).await?;
        Ok(inputs)
    }

    // LiteLLM's update_in_memory_litellm_params: the new timeout takes the same fallback as the start argument
    pub fn update_params(&mut self, params: &LitellmParams) -> Result<(), ZscalerAiGuardError> {
        self.settings.timeout = settings::resolve_timeout(params.base.fields.timeout)?;
        Ok(())
    }

    async fn send(
        &self,
        direction: Direction,
        content: &str,
        policy_id: &GuardrailValue,
        user: &UserKeyHeaders,
    ) -> Result<(), ZscalerAiGuardError> {
        let body = PolicyRequest {
            direction,
            content,
            policy_id: metadata::policy_id_wire(policy_id)?,
        };
        let headers = self.prepare_headers(user)?;
        let mut response = self
            .client
            .post(self.settings.url.as_str())
            .headers(headers)
            .json(&body)
            .timeout(self.settings.timeout)
            .send()
            .await
            .map_err(|source| ZscalerAiGuardError::Request { source })?;
        let status = response.status();
        let bytes = read_bounded(&mut response, MAX_RESPONSE_BYTES)
            .await
            .map_err(|error| match error {
                ReadError::TooLarge { limit } => ZscalerAiGuardError::ResponseTooLarge { limit },
                ReadError::Failed(source) => ZscalerAiGuardError::Request { source },
            })?;
        handle_response(status, &bytes)
    }

    fn prepare_headers(&self, user: &UserKeyHeaders) -> Result<HeaderMap, ZscalerAiGuardError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let bearer = match &self.settings.api_key {
            Some(key) => format!("Bearer {}", key.expose_secret()),
            None => "Bearer ".to_owned(),
        };
        let mut authorization =
            HeaderValue::from_str(&bearer).map_err(|source| ZscalerAiGuardError::Header {
                name: "authorization",
                source,
            })?;
        authorization.set_sensitive(true);
        headers.insert(AUTHORIZATION, authorization);
        insert_header(&mut headers, "user-api-key-alias", user.alias.as_deref())?;
        insert_header(
            &mut headers,
            "user-api-key-team-id",
            user.team_id.as_deref(),
        )?;
        insert_header(
            &mut headers,
            "user-api-key-user-id",
            user.user_id.as_deref(),
        )?;
        Ok(headers)
    }
}

fn insert_header(
    headers: &mut HeaderMap,
    name: &'static str,
    value: Option<&str>,
) -> Result<(), ZscalerAiGuardError> {
    let Some(value) = value else {
        return Ok(());
    };
    let value = HeaderValue::from_str(value)
        .map_err(|source| ZscalerAiGuardError::Header { name, source })?;
    headers.insert(HeaderName::from_static(name), value);
    Ok(())
}

fn handle_response(status: StatusCode, body: &[u8]) -> Result<(), ZscalerAiGuardError> {
    if status != StatusCode::OK {
        return Err(status_failure(status));
    }
    let answer: PolicyResponse = serde_json::from_slice(body)
        .map_err(|source| ZscalerAiGuardError::ResponseJson { source })?;
    evaluate_answer(&answer)
}

// LiteLLM's HTTP client raises on every 4xx and 5xx before _handle_response runs, so those arrive as a 500
fn status_failure(status: StatusCode) -> ZscalerAiGuardError {
    if !status.is_client_error() && !status.is_server_error() {
        return ZscalerAiGuardError::Http {
            status,
            reason: format!("Response status code: {}", status.as_u16()),
        };
    }
    let reason = if status == StatusCode::TOO_MANY_REQUESTS {
        "Rate limit reached. status_code: 429".to_owned()
    } else if status.is_server_error() {
        format!("Service is unavailable (HTTP {})", status.as_u16())
    } else {
        format!("Response status code: {}", status.as_u16())
    };
    ZscalerAiGuardError::Http {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        reason,
    }
}

fn evaluate_answer(answer: &PolicyResponse) -> Result<(), ZscalerAiGuardError> {
    if !status_code_is_200(answer.status_code.as_ref()) {
        return Err(ZscalerAiGuardError::Http {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            reason: status_code_reason(answer),
        });
    }
    match_answer(answer)
}

fn status_code_reason(answer: &PolicyResponse) -> String {
    format!(
        "statusCode in response: {}, errorMsg: {}",
        answer
            .status_code
            .as_ref()
            .map_or_else(|| "None".to_owned(), str_text),
        answer
            .error_message
            .as_ref()
            .map_or_else(|| "None".to_owned(), str_text),
    )
}

fn match_answer(answer: &PolicyResponse) -> Result<(), ZscalerAiGuardError> {
    if matches!(
        answer.action.as_ref(),
        Some(GuardrailValue::Text(action)) if action == "BLOCK"
    ) {
        return Err(ZscalerAiGuardError::Blocked {
            info: blocking_info(answer)?,
        });
    }
    let allowed = matches!(
        answer.action.as_ref(),
        Some(GuardrailValue::Text(action)) if action == "ALLOW" || action == "DETECT"
    );
    if allowed {
        return Ok(());
    }
    Err(ZscalerAiGuardError::Http {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        reason: format!(
            "Action field in response is {}, expecting 'ALLOW', 'BLOCK' or 'DETECT'",
            answer
                .action
                .as_ref()
                .map_or_else(|| "None".to_owned(), str_text),
        ),
    })
}

fn status_code_is_200(status_code: Option<&GuardrailValue>) -> bool {
    match status_code {
        Some(GuardrailValue::SignedInteger(200)) | Some(GuardrailValue::UnsignedInteger(200)) => {
            true
        }
        Some(GuardrailValue::Decimal(value)) => *value == 200.0,
        Some(GuardrailValue::Null)
        | Some(GuardrailValue::Boolean(_))
        | Some(GuardrailValue::Text(_))
        | Some(GuardrailValue::Array(_))
        | Some(GuardrailValue::Object(_))
        | None => false,
    }
}

fn blocking_info(answer: &PolicyResponse) -> Result<String, ZscalerAiGuardError> {
    let transaction = answer
        .transaction_id
        .as_ref()
        .map_or_else(|| "None".to_owned(), repr_text);
    let mut detectors: Vec<String> = Vec::new();
    for (name, details) in &answer.detector_responses {
        let GuardrailValue::Object(fields) = details else {
            return Err(ZscalerAiGuardError::DetectorShape);
        };
        let blocking = matches!(
            fields.get("action"),
            Some(GuardrailValue::Text(action)) if action == "BLOCK"
        );
        if blocking {
            detectors.push(quote_text(name));
        }
    }
    Ok(format!(
        "{{'transactionId': {transaction}, 'blockingDetectors': [{}]}}",
        detectors.join(", ")
    ))
}
