use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_http::proxy::limits::{ReadError, read_bounded};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;
use serde::Serialize;

use super::error::QualifireError;
use super::settings::{Check, OnFlagged};
use super::wire::{EvaluatePayload, InvokePayload, Payload, Verdict};
use super::{MAX_RESPONSE_BYTES, QualifireGuardrail};
use crate::generic_guardrail_contracts::{GuardrailApiInputs, GuardrailInputType};
use crate::qualifire::convert::{self, ApiMessage, ApiTool, MessageRow};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Overrides {
    pub evaluation_id: Option<String>,
    pub assertions: Option<Vec<String>>,
    pub on_flagged: Option<OnFlagged>,
}

pub(super) struct CheckRequest<'payload> {
    pub rows: &'payload [MessageRow],
    pub output: Option<&'payload str>,
    pub evaluation_id: Option<&'payload str>,
    pub assertions: &'payload [String],
    pub available_tools: Option<&'payload GuardrailValue>,
}

pub(super) struct Plan<'payload> {
    pub url: String,
    input: String,
    messages: Vec<ApiMessage>,
    tools: Option<Vec<ApiTool>>,
    output: Option<&'payload str>,
    evaluation_id: Option<&'payload str>,
    assertions: &'payload [String],
    checks: &'payload BTreeSet<Check>,
}

impl Plan<'_> {
    pub(super) fn payload(&self) -> Payload<'_> {
        let available_tools = self.tools.as_deref();
        if let Some(evaluation_id) = self.evaluation_id {
            return Payload::Invoke(InvokePayload {
                evaluation_id,
                input: &self.input,
                output: self.output.unwrap_or_default(),
                messages: &self.messages,
                available_tools,
            });
        }
        let tool_check = self.checks.contains(&Check::ToolSelectionQuality);
        Payload::Evaluate(EvaluatePayload {
            messages: &self.messages,
            output: self.output,
            prompt_injections: self
                .checks
                .contains(&Check::PromptInjections)
                .then_some(true),
            hallucinations_check: self.checks.contains(&Check::Hallucinations).then_some(true),
            grounding_check: self.checks.contains(&Check::Grounding).then_some(true),
            pii_check: self.checks.contains(&Check::Pii).then_some(true),
            content_moderation_check: self
                .checks
                .contains(&Check::ContentModeration)
                .then_some(true),
            tool_selection_quality_check: (tool_check && self.tools.is_some()).then_some(true),
            available_tools: available_tools.filter(|_tools| tool_check),
            assertions: (!self.assertions.is_empty()).then_some(self.assertions),
        })
    }
}

impl QualifireGuardrail {
    pub(super) fn plan<'payload>(
        &self,
        check: &CheckRequest<'payload>,
    ) -> Result<Plan<'payload>, QualifireError> {
        let messages = convert::api_messages(check.rows)?;
        let tools = convert::api_tools(check.available_tools)?;
        let base = &self.settings.api_base;
        let (url, input) = match check.evaluation_id {
            Some(_) => (
                format!("{base}/api/evaluation/invoke"),
                invocation_input(check.rows),
            ),
            None => {
                let tool_check = self.settings.checks.contains(&Check::ToolSelectionQuality);
                if tool_check && tools.is_none() {
                    tracing::debug!(
                        "Qualifire Guardrail: tool_selection_quality_check enabled but no available_tools provided, skipping this check"
                    );
                }
                (format!("{base}/api/evaluation/evaluate"), String::new())
            }
        };
        Ok(Plan {
            url,
            input,
            messages,
            tools,
            output: check.output,
            evaluation_id: check.evaluation_id,
            assertions: check.assertions,
            checks: &self.settings.checks,
        })
    }

    pub(super) async fn send(
        &self,
        url: &str,
        payload: &Payload<'_>,
    ) -> Result<Verdict, QualifireError> {
        tracing::debug!("Qualifire Guardrail: Making request to {url}");
        let request = self
            .client
            .post(url)
            .headers(self.headers()?)
            .json(payload)
            .timeout(self.settings.timeout);
        let mut response = request.send().await.map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(QualifireError::HttpStatus { status });
        }
        let body = read_bounded(&mut response, MAX_RESPONSE_BYTES)
            .await
            .map_err(read_error)?;
        serde_json::from_slice(&body).map_err(|source| QualifireError::ResponseJson { source })
    }

    fn headers(&self) -> Result<HeaderMap, QualifireError> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let key = self
            .settings
            .api_key
            .as_ref()
            .map_or("", |secret| secret.expose_secret().as_str());
        let mut value = HeaderValue::from_str(key)
            .map_err(|source| QualifireError::InvalidHeader { source })?;
        value.set_sensitive(true);
        headers.insert(HeaderName::from_static("x-qualifire-api-key"), value);
        Ok(headers)
    }
}

pub(super) fn overrides(
    dynamic: &BTreeMap<String, GuardrailValue>,
) -> Result<Overrides, QualifireError> {
    Ok(Overrides {
        evaluation_id: non_empty_text(dynamic.get("evaluation_id"), "evaluation_id")?,
        assertions: assertions(dynamic.get("assertions"))?,
        on_flagged: match dynamic.get("on_flagged") {
            None | Some(GuardrailValue::Null) => None,
            Some(GuardrailValue::Text(text)) if text.is_empty() => None,
            Some(GuardrailValue::Text(text)) => {
                Some(OnFlagged::parse(text).unwrap_or(OnFlagged::Block))
            }
            Some(_wrong) => {
                return Err(QualifireError::InvalidDynamicParam {
                    field: "on_flagged",
                });
            }
        },
    })
}

fn non_empty_text(
    value: Option<&GuardrailValue>,
    field: &'static str,
) -> Result<Option<String>, QualifireError> {
    match value {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) if text.is_empty() => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(text.clone())),
        Some(_wrong) => Err(QualifireError::InvalidDynamicParam { field }),
    }
}

fn assertions(value: Option<&GuardrailValue>) -> Result<Option<Vec<String>>, QualifireError> {
    let Some(present) = value else {
        return Ok(None);
    };
    let GuardrailValue::Array(items) = present else {
        return Err(QualifireError::InvalidDynamicParam {
            field: "assertions",
        });
    };
    let mut taken = Vec::with_capacity(items.len());
    for item in items {
        let GuardrailValue::Text(text) = item else {
            return Err(QualifireError::InvalidDynamicParam {
                field: "assertions",
            });
        };
        taken.push(text.clone());
    }
    Ok((!taken.is_empty()).then_some(taken))
}

// LiteLLM reads the last user message's content only when it is a string, and stops at that message
pub(super) fn invocation_input(rows: &[MessageRow]) -> String {
    for row in rows.iter().rev() {
        let is_user = matches!(
            row.get("role"),
            Some(GuardrailValue::Text(role)) if role.as_str() == "user"
        );
        if is_user {
            return match row.get("content") {
                Some(GuardrailValue::Text(text)) => text.clone(),
                _ => String::new(),
            };
        }
    }
    String::new()
}

pub(super) fn select_rows(
    inputs: &GuardrailApiInputs,
    request_data: &BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
) -> Result<Option<(Vec<MessageRow>, Option<String>)>, QualifireError> {
    let mut rows = match &inputs.structured_messages {
        Some(messages) if !messages.is_empty() => convert::message_rows(messages),
        _ => request_rows(request_data)?,
    };
    let texts = inputs.texts.as_deref().unwrap_or_default();
    let output = match input_type {
        GuardrailInputType::Response => texts.last().cloned(),
        GuardrailInputType::Request => None,
    };
    if input_type == GuardrailInputType::Response && rows.is_empty() && !texts.is_empty() {
        let content = output.clone().unwrap_or_default();
        rows = vec![message_row("assistant", &content)];
    }
    if rows.is_empty() {
        let Some(text) = texts.last() else {
            return Ok(None);
        };
        rows = vec![message_row("user", text)];
    }
    Ok(Some((rows, output)))
}

fn message_row(role: &str, content: &str) -> MessageRow {
    BTreeMap::from([
        ("role".to_owned(), GuardrailValue::Text(role.to_owned())),
        (
            "content".to_owned(),
            GuardrailValue::Text(content.to_owned()),
        ),
    ])
}

fn request_rows(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<Vec<MessageRow>, QualifireError> {
    match request_data.get("messages") {
        None | Some(GuardrailValue::Null) => Ok(Vec::new()),
        Some(GuardrailValue::Array(messages)) => messages.iter().map(row_from_value).collect(),
        Some(_wrong) => Err(QualifireError::InvalidField { field: "messages" }),
    }
}

fn row_from_value(value: &GuardrailValue) -> Result<MessageRow, QualifireError> {
    match value {
        GuardrailValue::Object(row) => Ok(row.clone()),
        _ => Err(QualifireError::InvalidField { field: "messages" }),
    }
}

fn request_error(source: reqwest::Error) -> QualifireError {
    if source.is_timeout() {
        QualifireError::Timeout { source }
    } else {
        QualifireError::Request { source }
    }
}

fn read_error(error: ReadError) -> QualifireError {
    match error {
        ReadError::TooLarge { limit } => QualifireError::ResponseTooLarge { limit },
        ReadError::Failed(source) => request_error(source),
    }
}
