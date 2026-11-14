#![expect(
    clippy::disallowed_types,
    reason = "the http command's request and response bodies are JSON values the caller and the proxy define, not a shape betterlite models"
)]

use std::io::Write;

use serde_json::Value;

use crate::commands::Payload;

use super::HttpError;

pub(super) fn request_payload(
    json: Option<&str>,
    data: Option<&str>,
    keep_content_type: bool,
) -> Result<Payload, HttpError> {
    if let Some(text) = json {
        let value: Value =
            serde_json::from_str(text).map_err(|source| HttpError::InvalidJson { source })?;
        let body = serde_json::to_vec(&value).map_err(|source| HttpError::Encode { source })?;
        return Ok(json_payload(body, keep_content_type));
    }
    let Some(text) = data else {
        return Ok(Payload::Empty);
    };
    match serde_json::from_str::<Value>(text) {
        Ok(value) => {
            let body = serde_json::to_vec(&value).map_err(|source| HttpError::Encode { source })?;
            Ok(json_payload(body, keep_content_type))
        }
        Err(_not_json) => Ok(Payload::Raw(text.as_bytes().to_vec())),
    }
}

const fn json_payload(body: Vec<u8>, keep_content_type: bool) -> Payload {
    if keep_content_type {
        return Payload::Raw(body);
    }
    Payload::Json(body)
}

pub(super) fn write_pretty(body: &[u8], stdout: &mut dyn Write) -> Result<(), HttpError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|source| HttpError::Response { source })?;
    let text =
        serde_json::to_string_pretty(&value).map_err(|source| HttpError::Encode { source })?;
    let _wrote_body = stdout.write_all(text.as_bytes());
    let _wrote_newline = stdout.write_all(b"\n");
    Ok(())
}
