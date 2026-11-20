use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::output_limits::{ErrorBody, without_rejected_fields};

use super::OpenAi;
use crate::operation::{OperationError, OperationResult};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedChat, PreparedRequest};
use crate::transform_retry::TransformOnUnprocessable;

impl TransformOnUnprocessable for OpenAi {
    fn should_retry(&self, response: &UpstreamResponse, _deployment: &Deployment) -> bool {
        response.status == 422
    }

    fn transform_request(
        &self,
        prepared: &mut PreparedRequest,
        response: &UpstreamResponse,
        _deployment: &Deployment,
    ) -> Result<(), OperationError> {
        let (PreparedRequest::Chat(chat) | PreparedRequest::Stream(chat)) = prepared else {
            return Ok(());
        };
        let sent = serde_json::from_slice(&chat.http.body)
            .map_err(|source| OperationError::Encode { source })?;
        let exception = serde_json::to_vec(&sdk_exception_body(&response.body))
            .map_err(|source| OperationError::Encode { source })?;
        // without_rejected_fields takes serde_json's own map and value, so both types come from these calls
        let error_body = serde_json::from_slice(&exception)
            .map_err(|source| OperationError::Encode { source })?;
        let kept = without_rejected_fields(ErrorBody::Sdk(&error_body), &sent)
            .map_err(|source| OperationError::RejectedFields { source })?;
        chat.http.body =
            serde_json::to_vec(&kept).map_err(|source| OperationError::Encode { source })?;
        Ok(())
    }
}

// openai-python's exception body: a JSON object's error member, else the JSON itself, else the body text
fn sdk_exception_body(body: &[u8]) -> JsonValue {
    match serde_json::from_slice(body) {
        Ok(JsonValue::Object(mut fields)) => match fields.shift_remove("error") {
            Some(error) => error,
            None => JsonValue::Object(fields),
        },
        Ok(other) => other,
        Err(_not_json) => JsonValue::Text(String::from_utf8(body.to_vec()).unwrap_or_default()),
    }
}

// LiteLLM's is_output_token_limit_error over an httpx.HTTPStatusError's response bytes
pub(super) fn output_limit(
    chat: &PreparedChat,
    response: &UpstreamResponse,
) -> Result<Option<OperationResult>, OperationError> {
    if response.status != 400 || !ErrorBody::Http(&response.body).reaches_output_limit() {
        return Ok(None);
    }
    let sent: JsonValue = serde_json::from_slice(&chat.http.body)
        .map_err(|source| OperationError::Encode { source })?;
    Ok(Some(OperationResult::OutputLimit {
        sent: Box::new(sent),
    }))
}

mod tests;
