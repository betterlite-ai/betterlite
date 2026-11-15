use betterlite_openai::openai_base_config::forwarding::{OpenAiErrorParts, OpenAiErrorRecord};
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use super::error::MaiImageError;
use super::params::{DEFAULT_HEIGHT, DEFAULT_WIDTH};
use super::usage::normalize_mai_image_usage;

pub struct PostCall<'call> {
    pub input: &'call str,
    pub api_key: Option<&'call SecretString>,
    pub complete_input_dict: &'call Object,
    pub original_response: &'call Object,
}

// B249's post_call is injected, so this crate never builds a logging state of its own
pub trait ImageLogging {
    fn post_call(&mut self, call: PostCall<'_>);
}

pub struct MaiImageReply<'reply> {
    pub body: &'reply [u8],
    pub status_code: u16,
    pub request_data: &'reply Object,
    pub optional_params: &'reply Object,
    pub api_key: Option<&'reply SecretString>,
}

// B004 projects body as an image_generation response and the size is set on it afterwards
#[derive(Debug, Clone, PartialEq)]
pub struct MaiImageResponse {
    pub body: Object,
    pub size: String,
}

// A body that is not JSON becomes B668's OpenAIError, carrying the raw text and the reply's status
pub fn decode_mai_image_reply(body: &[u8], status_code: u16) -> Result<Object, MaiImageError> {
    let parsed: Json = serde_json::from_slice(body).map_err(|source| MaiImageError::NotJson {
        record: Box::new(OpenAiErrorRecord::from_parts(OpenAiErrorParts {
            status_code,
            message: decode_replacing_invalid(body),
            headers: None,
            request: None,
            response: None,
            body: None,
        })),
        source,
    })?;
    let Json::Object(mut response) = parsed else {
        return Err(MaiImageError::NotAnObject);
    };
    // Usage is only normalized when the key is there, and an explicit null gets the zeroed shape
    if let Some(usage) = response.get("usage") {
        let normalized = match usage {
            Json::Null => normalize_mai_image_usage(None)?,
            Json::Object(fields) => normalize_mai_image_usage(Some(fields))?,
            Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                return Err(MaiImageError::FieldType { field: "usage" });
            }
        };
        response.insert("usage".to_owned(), Json::Object(normalized));
    }
    Ok(response)
}

pub fn transform_image_reply(
    reply: &MaiImageReply<'_>,
    logging: &mut dyn ImageLogging,
) -> Result<MaiImageResponse, MaiImageError> {
    let body = decode_mai_image_reply(reply.body, reply.status_code)?;
    let input = match reply.request_data.get("prompt") {
        None | Some(Json::Null) => "",
        Some(Json::Text(prompt)) => prompt,
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(MaiImageError::FieldType { field: "prompt" });
        }
    };
    logging.post_call(PostCall {
        input,
        api_key: reply.api_key,
        complete_input_dict: reply.request_data,
        original_response: &body,
    });
    // The size comes from the request's width and height, whatever the returned images say
    let width = dimension_text(reply.optional_params, "width", DEFAULT_WIDTH)?;
    let height = dimension_text(reply.optional_params, "height", DEFAULT_HEIGHT)?;
    Ok(MaiImageResponse {
        body,
        size: format!("{width}x{height}"),
    })
}

// Python's errors="replace": one U+FFFD per invalid sequence, which clippy.toml keeps String::from_utf8_lossy from doing
fn decode_replacing_invalid(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push('\u{FFFD}');
        }
    }
    text
}

fn dimension_text(
    optional_params: &Object,
    key: &'static str,
    default: i64,
) -> Result<String, MaiImageError> {
    match optional_params.get(key) {
        None | Some(Json::Null) => Ok(default.to_string()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Number(number)) => Ok(number.to_string()),
        Some(Json::Bool(flag)) => Ok(flag.to_string()),
        Some(Json::List(_) | Json::Object(_)) => Err(MaiImageError::FieldType { field: key }),
    }
}
