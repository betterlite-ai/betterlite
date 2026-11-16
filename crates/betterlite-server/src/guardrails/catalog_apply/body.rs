use std::collections::BTreeMap;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::routes::ApplyGuardrailRequest;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::error::Category;
use serde_path_to_error::{Path, Segment};

use super::ports::Body;

const FIELD_REQUIRED: &str = "Field required";
const JSON_DECODE_ERROR: &str = "JSON decode error";

// FastAPI's 422 for a body that never reached the declared model
pub enum BodyError {
    Missing,
    Fields(serde_path_to_error::Error<serde_json::Error>),
    Trailing,
}

impl IntoResponse for BodyError {
    fn into_response(self) -> Response {
        match self {
            Self::Missing => validation_response(ValidationIssue {
                issue_type: "missing",
                loc: body_location(None),
                msg: FIELD_REQUIRED.to_owned(),
            }),
            Self::Fields(source) => json_body_response(&source),
            Self::Trailing => invalid_json_response(),
        }
    }
}

// a blank body and a JSON null are both the missing body FastAPI reports for a required model
pub(super) fn decode_body<T>(body: &[u8]) -> Result<T, BodyError>
where
    T: DeserializeOwned,
{
    if body.iter().all(u8::is_ascii_whitespace) {
        return Err(BodyError::Missing);
    }
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let value = serde_path_to_error::deserialize::<_, Option<T>>(&mut deserializer)
        .map_err(BodyError::Fields)?;
    deserializer
        .end()
        .map_err(|_trailing| BodyError::Trailing)?;
    value.ok_or(BodyError::Missing)
}

fn json_body_response(source: &serde_path_to_error::Error<serde_json::Error>) -> Response {
    let message = source.inner().to_string();
    if let Some(field) = message
        .strip_prefix("missing field `")
        .and_then(|rest| rest.split_once('`'))
        .map(|(field, _position)| field)
    {
        let mut loc = body_location(Some(source.path()));
        loc.push(LocPart::Field(field.to_owned()));
        return validation_response(ValidationIssue {
            issue_type: "missing",
            loc,
            msg: FIELD_REQUIRED.to_owned(),
        });
    }
    let issue_type = match source.inner().classify() {
        Category::Syntax | Category::Eof => return invalid_json_response(),
        Category::Io | Category::Data => "value_error",
    };
    validation_response(ValidationIssue {
        issue_type,
        loc: body_location(Some(source.path())),
        msg: message,
    })
}

fn invalid_json_response() -> Response {
    validation_response(ValidationIssue {
        issue_type: "json_invalid",
        loc: body_location(None),
        msg: JSON_DECODE_ERROR.to_owned(),
    })
}

fn validation_response(issue: ValidationIssue) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(ValidationBody { detail: [issue] }),
    )
        .into_response()
}

fn body_location(path: Option<&Path>) -> Vec<LocPart> {
    let mut loc = vec![LocPart::Field("body".to_owned())];
    let Some(segments) = path else {
        return loc;
    };
    for segment in segments {
        match segment {
            Segment::Seq { index } => loc.push(LocPart::Index(*index)),
            Segment::Map { key } => loc.push(LocPart::Field(key.clone())),
            Segment::Enum { variant } => loc.push(LocPart::Field(variant.clone())),
            Segment::Unknown => {}
        }
    }
    loc
}

#[derive(Serialize)]
struct ValidationBody {
    detail: [ValidationIssue; 1],
}

#[derive(Debug, Serialize)]
struct ValidationIssue {
    #[serde(rename = "type")]
    issue_type: &'static str,
    loc: Vec<LocPart>,
    msg: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum LocPart {
    Field(String),
    Index(usize),
}

// LiteLLM's data dict, the request_data handed to the guardrail, and the inputs of its call
pub(super) fn call_data(request: &ApplyGuardrailRequest) -> Body {
    let messages = request.messages.as_deref().unwrap_or_default();
    let route = GuardrailValue::Text("/apply_guardrail".to_owned());
    let metadata = BTreeMap::from([("route".to_owned(), route)]);
    let mut data = Body::new();
    data.insert(
        "guardrail_name".to_owned(),
        GuardrailValue::Text(request.guardrail_name.clone()),
    );
    data.insert(
        "input".to_owned(),
        GuardrailValue::Array(vec![GuardrailValue::Text(request.text.clone())]),
    );
    data.insert(
        "messages".to_owned(),
        GuardrailValue::Array(message_values(messages)),
    );
    data.insert("metadata".to_owned(), GuardrailValue::Object(metadata));
    data
}

pub(super) fn sent_request_data(request: &ApplyGuardrailRequest) -> Body {
    let mut sent = Body::new();
    if let Some(messages) = &request.messages {
        sent.insert(
            "messages".to_owned(),
            GuardrailValue::Array(message_values(messages)),
        );
    }
    if let Some(metadata) = &request.metadata {
        sent.insert(
            "metadata".to_owned(),
            GuardrailValue::Object(metadata.0.clone()),
        );
    }
    sent
}

fn message_values(messages: &[OpaqueParams]) -> Vec<GuardrailValue> {
    messages
        .iter()
        .map(|message| GuardrailValue::Object(message.0.clone()))
        .collect()
}

pub(super) fn call_inputs(text: &str) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(vec![text.to_owned()]),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}
