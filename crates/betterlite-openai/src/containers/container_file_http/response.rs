use serde::de::DeserializeOwned;
use serde_json::Value;

use super::transport::ContainerHttpReply;
use super::{ContainerFileHttpError, ContainerFileReply};
use crate::api::container_contracts::catalog::ContainerEndpoint;
use crate::api::container_contracts::config::BaseContainerConfig;
use crate::api::container_contracts::error::BaseLlmException;

// httpx.codes.is_error: only 4xx and 5xx count, so a 3xx answer goes on to the body
const FIRST_ERROR_STATUS: u16 = 400;
const LAST_ERROR_STATUS: u16 = 599;

// clippy.toml refuses lossy decoding, so a body that is not UTF-8 is named by its size
fn body_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}

// The error object's message when the body is JSON with one, the body text otherwise
#[must_use]
pub fn error_message_from_response(body: &[u8]) -> String {
    if let Ok(Value::Object(fields)) = serde_json::from_slice::<Value>(body)
        && let Some(Value::Object(error)) = fields.get("error")
        && let Some(Value::String(message)) = error.get("message")
    {
        return message.clone();
    }
    body_text(body)
}

pub fn raise_for_error_status(
    config: &dyn BaseContainerConfig,
    reply: &ContainerHttpReply,
) -> Result<(), BaseLlmException> {
    if !(FIRST_ERROR_STATUS..=LAST_ERROR_STATUS).contains(&reply.status) {
        return Ok(());
    }
    Err(config.get_error_class(
        &error_message_from_response(&reply.body),
        reply.status,
        &reply.headers,
    ))
}

fn model<Model>(json: Value, response_type: &str) -> Result<Model, ContainerFileHttpError>
where
    Model: DeserializeOwned,
{
    serde_json::from_value(json).map_err(|source| ContainerFileHttpError::Model {
        response_type: response_type.to_owned(),
        source,
    })
}

pub fn transform_reply(
    config: &dyn BaseContainerConfig,
    endpoint: &ContainerEndpoint,
    reply: &ContainerHttpReply,
) -> Result<ContainerFileReply, ContainerFileHttpError> {
    raise_for_error_status(config, reply)?;
    if endpoint.returns_binary {
        return Ok(ContainerFileReply::Bytes(reply.body.clone()));
    }
    let json: Value = serde_json::from_slice(&reply.body)
        .map_err(|source| ContainerFileHttpError::Json { source })?;
    // a 2xx body can still carry an error, and it raises with the status it came with
    if let Value::Object(fields) = &json
        && let Some(error) = fields.get("error")
    {
        let message = match error.get("message") {
            Some(Value::String(text)) => text.clone(),
            Some(_) | None => json.to_string(),
        };
        return Err(ContainerFileHttpError::from(config.get_error_class(
            &message,
            reply.status,
            &reply.headers,
        )));
    }
    let response_type = endpoint.response_type.as_str();
    match response_type {
        "ContainerFileListResponse" => model(json, response_type).map(ContainerFileReply::List),
        "ContainerFileObject" => model(json, response_type).map(ContainerFileReply::File),
        "DeleteContainerFileResponse" => {
            model(json, response_type).map(ContainerFileReply::Deleted)
        }
        _ => Ok(ContainerFileReply::Raw(json)),
    }
}
