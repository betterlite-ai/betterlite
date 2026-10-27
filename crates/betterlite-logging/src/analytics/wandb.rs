use super::wire::{Choice, KnownObject, Object, Request, Response};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TraceResult {
    pub inputs: BTreeMap<String, String>,
    pub outputs: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Span<Fields> {
    pub name: String,
    pub attributes: Response<Fields>,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
    pub span_kind: String,
    pub results: Vec<TraceResult>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Model<RequestFields, ResponseFields> {
    pub request: Request<RequestFields>,
    pub response: Response<ResponseFields>,
    #[serde(rename = "_kind")]
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TraceTree<RequestFields, ResponseFields> {
    pub root_span: Span<ResponseFields>,
    pub model_dict: Model<RequestFields, ResponseFields>,
}

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("missing {field} in analytics request or response")]
    Missing { field: &'static str },
    #[error("analytics timestamp is out of range")]
    Timestamp,
    #[error("cannot convert analytics timestamp")]
    Conversion {
        #[source]
        source: rust_decimal::Error,
    },
    #[error("cannot format analytics messages")]
    Format {
        #[source]
        source: fmt::Error,
    },
}

pub fn resolve<RequestFields, ResponseFields>(
    request: &Request<RequestFields>,
    response: &Response<ResponseFields>,
    elapsed: Decimal,
) -> Option<TraceTree<RequestFields, ResponseFields>>
where
    RequestFields: Clone,
    ResponseFields: Clone,
{
    let object = match &response.object {
        Object::Known(object) => *object,
        Object::Unknown(name) => {
            tracing::debug!(object = name, "Unknown OpenAI response object");
            return None;
        }
    };
    match project(request, response, elapsed, object) {
        Ok(trace) => Some(trace),
        Err(error) => {
            tracing::warn!(%error, "Failed to resolve request/response");
            None
        }
    }
}

fn project<RequestFields, ResponseFields>(
    request: &Request<RequestFields>,
    response: &Response<ResponseFields>,
    elapsed: Decimal,
    object: KnownObject,
) -> Result<TraceTree<RequestFields, ResponseFields>, ResolveError>
where
    RequestFields: Clone,
    ResponseFields: Clone,
{
    let (request_text, style) = format_request(request, object)?;
    let results = response
        .choices
        .iter()
        .map(|choice| {
            let text = format_choice(choice, &style)?;
            Ok(TraceResult {
                inputs: BTreeMap::from([("request".to_owned(), request_text.clone())]),
                outputs: BTreeMap::from([("response".to_owned(), text)]),
            })
        })
        .collect::<Result<Vec<_>, ResolveError>>()?;
    let start_time_ms = milliseconds(response.created)?;
    let end_time_ms = start_time_ms
        .checked_add(milliseconds(elapsed)?)
        .ok_or(ResolveError::Timestamp)?;
    let kind = match object {
        KnownObject::Edit => "edit",
        KnownObject::Completion => "text_completion",
        KnownObject::Chat => "chat.completion",
    };
    let model = response.model.as_deref().unwrap_or("openai");
    Ok(TraceTree {
        root_span: Span {
            name: format!("{model}_{kind}_{}", response.created),
            attributes: response.clone(),
            start_time_ms,
            end_time_ms,
            span_kind: "LLM".to_owned(),
            results,
        },
        model_dict: Model {
            request: request.clone(),
            response: response.clone(),
            kind: "openai".to_owned(),
        },
    })
}

enum ChoiceStyle {
    Text(&'static str),
    Message,
}

fn format_request<Fields>(
    request: &Request<Fields>,
    object: KnownObject,
) -> Result<(String, ChoiceStyle), ResolveError> {
    match object {
        KnownObject::Edit => {
            let instruction = request.instruction.as_ref().ok_or(ResolveError::Missing {
                field: "instruction",
            })?;
            let input = request
                .input
                .as_ref()
                .ok_or(ResolveError::Missing { field: "input" })?;
            Ok((
                format!("\n\n**Instruction**: {instruction}\n\n**Input**: {input}\n"),
                ChoiceStyle::Text("Edited"),
            ))
        }
        KnownObject::Completion => {
            let prompt = request
                .prompt
                .as_ref()
                .ok_or(ResolveError::Missing { field: "prompt" })?;
            Ok((
                format!("\n\n**Prompt**: {prompt}\n"),
                ChoiceStyle::Text("Completion"),
            ))
        }
        KnownObject::Chat => {
            let messages = request
                .messages
                .as_ref()
                .ok_or(ResolveError::Missing { field: "messages" })?;
            let mut text = String::new();
            for message in messages {
                fmt::write(
                    &mut text,
                    format_args!("\n\n**{}**: {}\n", message.role, message.content),
                )
                .map_err(|source| ResolveError::Format { source })?;
            }
            Ok((text, ChoiceStyle::Message))
        }
    }
}

fn format_choice(choice: &Choice, style: &ChoiceStyle) -> Result<String, ResolveError> {
    let (label, text) = match style {
        ChoiceStyle::Text(label) => {
            let text = choice
                .text
                .as_deref()
                .ok_or(ResolveError::Missing { field: "text" })?;
            (*label, text)
        }
        ChoiceStyle::Message => {
            let message = choice
                .message
                .as_ref()
                .ok_or(ResolveError::Missing { field: "message" })?;
            (message.role.as_str(), message.content.as_str())
        }
    };
    Ok(format!("\n\n**{label}**: {text}\n"))
}

fn milliseconds(seconds: Decimal) -> Result<i64, ResolveError> {
    let scaled = seconds
        .checked_mul(Decimal::from(1000_u16))
        .ok_or(ResolveError::Timestamp)?;
    let rounded = scaled.round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven);
    i64::try_from(rounded).map_err(|source| ResolveError::Conversion { source })
}
