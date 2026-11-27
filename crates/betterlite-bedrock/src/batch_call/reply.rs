use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::embedding_jobs::error::FailureBody;

use super::error::BatchCallError;
use super::transport::{ControlPlaneReply, header};

// botocore reads an empty success body as {} and its structure parser fails on a null one
pub fn decode_reply(reply: &ControlPlaneReply) -> Result<Json, BatchCallError> {
    if reply.body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Json::Object(IndexMap::new()));
    }
    let body: Json =
        serde_json::from_slice(&reply.body).map_err(|source| BatchCallError::Decode { source })?;
    if matches!(body, Json::Null) {
        return Err(BatchCallError::ResponseShape);
    }
    Ok(body)
}

// a ClientError of 300 and up carries the header code first, then the body code, then __type
#[must_use]
pub fn client_error(reply: &ControlPlaneReply) -> BatchCallError {
    let text = String::from_utf8(reply.body.clone()).unwrap_or_default();
    let code = if reply.status < 301 {
        // botocore reads 300 as a success body, so its ClientError carries no Error at all
        None
    } else {
        let body: Json =
            serde_json::from_slice(&reply.body).unwrap_or(Json::Object(IndexMap::new()));
        error_code(reply, &body)
    };
    BatchCallError::Client {
        status: reply.status,
        code,
        body: FailureBody(text),
    }
}

fn error_code(reply: &ControlPlaneReply, body: &Json) -> Option<String> {
    if let Some(value) = header(&reply.headers, "x-amzn-errortype") {
        return Some(trim_code(value));
    }
    let Json::Object(fields) = body else {
        return Some(reply.status.to_string());
    };
    let coded = fields.get("code").or_else(|| fields.get("Code"));
    if let Some(code) = coded {
        return match code {
            Json::Null => base_code(fields, reply.status),
            Json::Text(text) => Some(trim_code(text)),
            Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                Some(text_of(code))
            }
        };
    }
    base_code(fields, reply.status)
}

fn base_code(fields: &IndexMap<String, Json>, status: u16) -> Option<String> {
    match fields.get("__type") {
        Some(Json::Text(text)) => Some(trim_code(text)),
        // an explicit null __type leaves botocore's initial empty Code in place
        Some(Json::Null) => Some(String::new()),
        None => Some(status.to_string()),
        Some(value) => Some(text_of(value)),
    }
}

fn trim_code(value: &str) -> String {
    let before_colon = value.split(':').next().unwrap_or(value);
    before_colon
        .rsplit('#')
        .next()
        .unwrap_or(before_colon)
        .to_owned()
}

// a code of another type is kept whole, and it never names one of the two tolerated exceptions
fn text_of(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Bool(flag) => if *flag { "True" } else { "False" }.to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Null => "None".to_owned(),
        Json::List(_) | Json::Object(_) => String::new(),
    }
}
