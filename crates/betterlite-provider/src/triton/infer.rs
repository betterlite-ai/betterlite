use betterlite_prompts::builtin::{Content, Field};
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_json::value::RawValue;

use super::datatype::TritonDatatype;
use super::error::TritonError;
use super::reply::{decode_reply, parse_object};
use super::request::TransformInput;

pub const DEFAULT_MAX_TOKENS: i64 = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InferRequest {
    pub inputs: Vec<InferInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InferInput {
    pub name: String,
    pub shape: [usize; 1],
    pub datatype: TritonDatatype,
    pub data: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InferReply {
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InferBody {
    outputs: Vec<Box<RawValue>>,
}

#[derive(Debug, Deserialize)]
struct InferOutput {
    #[serde(deserialize_with = "Option::deserialize")]
    data: Option<InferData>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum InferData {
    Text(String),
    Texts(Vec<String>),
}

pub fn transform_request(input: &TransformInput<'_>) -> Result<InferRequest, TritonError> {
    let Some(message) = input.messages.first() else {
        return Err(TritonError::NoMessages);
    };
    let text_input = InferInput {
        name: String::from("text_input"),
        shape: [1],
        datatype: TritonDatatype::Bytes,
        data: vec![content_field(message.content.clone())],
    };
    let mut inputs = vec![text_input];
    for (name, value) in input.optional_params {
        if name == "stream" || name == "max_retries" {
            continue;
        }
        inputs.push(InferInput {
            name: name.clone(),
            shape: [1],
            datatype: datatype_of(value),
            data: vec![value.clone()],
        });
    }
    if !input.optional_params.contains_key("max_tokens") {
        inputs.push(InferInput {
            name: String::from("max_tokens"),
            shape: [1],
            datatype: TritonDatatype::Int32,
            data: vec![Field::Number(Number::from(DEFAULT_MAX_TOKENS))],
        });
    }
    Ok(InferRequest { inputs })
}

pub fn transform_response(status_code: u16, body: &[u8]) -> Result<InferReply, TritonError> {
    let decoded: InferBody = decode_reply(status_code, body)?;
    let Some(first) = decoded.outputs.first() else {
        return Err(TritonError::NoOutputs);
    };
    let output: InferOutput =
        parse_object(first).map_err(|source| TritonError::ReplyShape { source })?;
    let text = match output.data {
        None => None,
        Some(InferData::Text(text)) => Some(text),
        Some(InferData::Texts(values)) => Some(values.concat()),
    };
    Ok(InferReply { text })
}

fn datatype_of(value: &Field) -> TritonDatatype {
    match value {
        Field::Number(number) if number.is_i64() || number.is_u64() => TritonDatatype::Int32,
        Field::Number(_) => TritonDatatype::Fp32,
        Field::Bool(_) => TritonDatatype::Int32,
        Field::Null | Field::Text(_) | Field::List(_) | Field::Object(_) => TritonDatatype::Bytes,
    }
}

fn content_field(content: Content) -> Field {
    match content {
        Content::Text(text) => Field::Text(text),
        Content::Parts(parts) => Field::List(
            parts
                .into_iter()
                .map(|part| Field::Object(part.fields))
                .collect(),
        ),
        Content::Null => Field::Null,
    }
}
