use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, HeaderValue};
use secrecy::ExposeSecret as _;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_json::value::RawValue;

use crate::typesafe::typesafe_failure_handling::{TypeSafeError, handle_failure};
use crate::typesafe::typesafe_instructions::question_instructions;
use crate::typesafe::typesafe_message_parsing::safe_response_text;
use crate::typesafe::typesafe_state_building::TypeSafeState;
use crate::typesafe::{DEFAULT_JEV_TIMEOUT, DEFAULT_RESPONSE_TEXT_LIMIT, TypeSafeGuardrail};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
enum NoulQuestionType {
    #[serde(rename = "noul")]
    Noul,
}

#[derive(Debug, Serialize)]
struct JevQuestion {
    #[serde(rename = "type")]
    kind: NoulQuestionType,
    instructions: String,
}

#[derive(Debug, Serialize)]
struct JevSystemOneRequest<'source> {
    model: &'source str,
    state: &'source TypeSafeState,
    questions: IndexMap<String, JevQuestion>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type")]
pub enum JevAnswer {
    #[serde(rename = "noul")]
    Noul { noul: f64 },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct JevSystemOneResponse {
    pub answers: IndexMap<String, JevAnswer>,
}

fn build_questions(question_ids: &[String]) -> IndexMap<String, JevQuestion> {
    let mut questions = IndexMap::with_capacity(question_ids.len());
    for question_id in question_ids {
        questions.insert(
            question_id.clone(),
            JevQuestion {
                kind: NoulQuestionType::Noul,
                instructions: question_instructions(question_id),
            },
        );
    }
    questions
}

fn failure_detail(error_type: &str, message: String) -> Object {
    let mut detail = Object::new();
    detail.insert("error_type".to_owned(), Json::Text(error_type.to_owned()));
    detail.insert("detail".to_owned(), Json::Text(message));
    detail
}

fn body_detail(body: &[u8]) -> Object {
    let mut detail = Object::new();
    detail.insert(
        "body".to_owned(),
        Json::Text(safe_response_text(Some(body), DEFAULT_RESPONSE_TEXT_LIMIT)),
    );
    detail
}

fn status_detail(status: StatusCode, extra: (&str, Json)) -> Object {
    let mut detail = Object::new();
    detail.insert(
        "status_code".to_owned(),
        Json::Number(Number::from(status.as_u16())),
    );
    detail.insert(extra.0.to_owned(), extra.1);
    detail
}

enum LimitedBody {
    Complete(Vec<u8>),
    TooLarge,
}

async fn read_limited_body(mut response: reqwest::Response) -> Result<LimitedBody, reqwest::Error> {
    let mut body: Vec<u8> = Vec::new();
    loop {
        let Some(chunk) = response.chunk().await? else {
            return Ok(LimitedBody::Complete(body));
        };
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Ok(LimitedBody::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
}

fn answers_are_valid(response: &JevSystemOneResponse) -> bool {
    response.answers.values().all(|answer| {
        let JevAnswer::Noul { noul } = answer;
        (0.0_f64..=1.0_f64).contains(noul)
    })
}

async fn send_request(
    guardrail: &TypeSafeGuardrail,
    request: &JevSystemOneRequest<'_>,
) -> Result<Option<reqwest::Response>, TypeSafeError> {
    let endpoint = format!(
        "{}/v1/systemone",
        guardrail.settings.api_base.trim_end_matches('/')
    );
    let mut authorization =
        match HeaderValue::from_str(&format!("Bearer {}", guardrail.api_key.expose_secret())) {
            Ok(value) => value,
            Err(source) => {
                handle_failure(
                    guardrail,
                    "TypeSafe evaluation service request failed",
                    &failure_detail("InvalidHeaderValue", source.to_string()),
                )?;
                return Ok(None);
            }
        };
    authorization.set_sensitive(true);
    let response = guardrail
        .client
        .post(endpoint)
        .header(AUTHORIZATION, authorization)
        .json(request)
        .timeout(guardrail.timeout.unwrap_or(DEFAULT_JEV_TIMEOUT))
        .send()
        .await;
    match response {
        Ok(value) => Ok(Some(value)),
        Err(source) => {
            handle_failure(
                guardrail,
                "TypeSafe evaluation service request failed",
                &failure_detail("reqwest::Error", source.to_string()),
            )?;
            Ok(None)
        }
    }
}

fn parse_response(
    guardrail: &TypeSafeGuardrail,
    status: StatusCode,
    body: &[u8],
) -> Result<Option<JevSystemOneResponse>, TypeSafeError> {
    if !status.is_success() {
        handle_failure(
            guardrail,
            "TypeSafe evaluation service returned an error",
            &status_detail(
                status,
                (
                    "body",
                    Json::Text(safe_response_text(Some(body), DEFAULT_RESPONSE_TEXT_LIMIT)),
                ),
            ),
        )?;
        return Ok(None);
    }
    let Ok(raw) = serde_json::from_slice::<Box<RawValue>>(body) else {
        handle_failure(
            guardrail,
            "TypeSafe evaluation service returned an unreadable response",
            &body_detail(body),
        )?;
        return Ok(None);
    };
    match serde_json::from_str::<JevSystemOneResponse>(raw.get()) {
        Ok(response) if answers_are_valid(&response) => Ok(Some(response)),
        Ok(_) | Err(_) => {
            handle_failure(
                guardrail,
                "TypeSafe evaluation service returned unexpected response shape",
                &body_detail(body),
            )?;
            Ok(None)
        }
    }
}

pub async fn call_systemone(
    guardrail: &TypeSafeGuardrail,
    state: &TypeSafeState,
    question_ids: &[String],
) -> Result<Option<JevSystemOneResponse>, TypeSafeError> {
    let request = JevSystemOneRequest {
        model: &guardrail.settings.model,
        state,
        questions: build_questions(question_ids),
    };
    let Some(response) = send_request(guardrail, &request).await? else {
        return Ok(None);
    };
    let status = response.status();
    let body = match read_limited_body(response).await {
        Ok(LimitedBody::Complete(body)) => body,
        Ok(LimitedBody::TooLarge) => {
            handle_failure(
                guardrail,
                "TypeSafe evaluation service returned an oversized response",
                &status_detail(
                    status,
                    ("limit", Json::Number(Number::from(MAX_RESPONSE_BYTES))),
                ),
            )?;
            return Ok(None);
        }
        Err(source) => {
            handle_failure(
                guardrail,
                "TypeSafe evaluation service returned an unreadable response",
                &failure_detail("reqwest::Error", source.to_string()),
            )?;
            return Ok(None);
        }
    };
    parse_response(guardrail, status, &body)
}

#[path = "typesafe_api_call/tests.rs"]
mod tests;
