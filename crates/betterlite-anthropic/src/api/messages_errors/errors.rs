use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnthropicErrorType {
    InvalidRequestError,
    AuthenticationError,
    PermissionError,
    NotFoundError,
    RequestTooLarge,
    RateLimitError,
    ApiError,
    OverloadedError,
}

impl AnthropicErrorType {
    fn for_status(status: u16) -> Self {
        match status {
            400 => Self::InvalidRequestError,
            401 => Self::AuthenticationError,
            403 => Self::PermissionError,
            404 => Self::NotFoundError,
            413 => Self::RequestTooLarge,
            429 => Self::RateLimitError,
            529 => Self::OverloadedError,
            _ => Self::ApiError,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnthropicErrorDetail {
    #[serde(rename = "type")]
    pub error_type: Value,
    pub message: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_specific_fields: Option<BTreeMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub litellm_call_id: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnthropicErrorResponse {
    #[serde(rename = "type")]
    pub response_type: Value,
    pub error: AnthropicErrorDetail,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

pub struct AnthropicExceptionMapping;

impl AnthropicExceptionMapping {
    #[must_use]
    pub fn get_error_type(status_code: u16) -> AnthropicErrorType {
        let error_type = AnthropicErrorType::for_status(status_code);
        error_type
    }

    #[must_use]
    pub fn create_error_response(
        status_code: u16,
        message: Value,
        request_id: Option<&str>,
    ) -> AnthropicErrorResponse {
        let response = Self::new_response(status_code, message, request_id);
        response
    }

    #[must_use]
    pub fn transform_to_anthropic_error(
        status_code: u16,
        raw_message: &str,
        request_id: Option<&str>,
    ) -> AnthropicErrorResponse {
        let parsed = serde_json::from_str::<Value>(raw_message).ok();
        Self::transform(status_code, raw_message, parsed, request_id)
    }

    pub fn transform(
        status_code: u16,
        raw_message: &str,
        parsed: Option<Value>,
        request_id: Option<&str>,
    ) -> AnthropicErrorResponse {
        if let Some(Value::Object(mut object)) = parsed {
            let passes = object.get("type").is_some_and(|value| value == "error")
                && object.get("error").is_some_and(|value| {
                    value.as_object().is_some_and(|error| {
                        error.contains_key("type") && error.contains_key("message")
                    })
                });
            if passes {
                if let Some(request_id) = request_id.filter(|value| !value.is_empty())
                    && !object.contains_key("request_id")
                {
                    object.insert(
                        "request_id".to_owned(),
                        Value::String(request_id.to_owned()),
                    );
                }
                if let Ok(response) = serde_json::from_value(Value::Object(object.clone())) {
                    return response;
                }
            }
            let message = if let Some(Value::Object(detail)) = object.get("detail") {
                detail
                    .get("message")
                    .cloned()
                    .unwrap_or_else(|| Value::String(raw_message.to_owned()))
            } else {
                object
                    .get("Message")
                    .and_then(Value::as_str)
                    .filter(|message| !message.is_empty())
                    .map(|message| Value::String(message.to_owned()))
                    .or_else(|| {
                        object
                            .get("message")
                            .and_then(Value::as_str)
                            .map(|message| Value::String(message.to_owned()))
                    })
                    .unwrap_or_else(|| Value::String(raw_message.to_owned()))
            };
            return Self::new_response(status_code, message, request_id);
        }
        Self::new_response(
            status_code,
            Value::String(raw_message.to_owned()),
            request_id,
        )
    }

    fn new_response(
        status_code: u16,
        message: Value,
        request_id: Option<&str>,
    ) -> AnthropicErrorResponse {
        let response = AnthropicErrorResponse {
            response_type: Value::String("error".to_owned()),
            error: AnthropicErrorDetail {
                error_type: serde_json::to_value(AnthropicErrorType::for_status(status_code))
                    .unwrap_or(Value::Null),
                message,
                provider_specific_fields: None,
                litellm_call_id: None,
                extra: BTreeMap::new(),
            },
            request_id: request_id
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_owned())),
            extra: BTreeMap::new(),
        };
        response
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnthropicErrorSseFrame {
    pub status_code: u16,
    pub error_response: AnthropicErrorResponse,
    body: String,
}

impl AnthropicErrorSseFrame {
    #[must_use]
    pub fn new(status_code: u16, error_response: AnthropicErrorResponse) -> Self {
        let body = serde_json::to_string(&error_response).unwrap_or_else(|_| "{}".to_owned());
        Self {
            status_code,
            error_response,
            body: format!("event: error\ndata: {body}\n\n"),
        }
    }

    #[must_use]
    pub fn json_body(&self, call_id: Option<&str>) -> AnthropicErrorResponse {
        let Some(call_id_value) = call_id else {
            return self.error_response.clone();
        };
        let mut response = self.error_response.clone();
        response.error.litellm_call_id = Some(call_id_value.to_owned());
        response
    }
}

impl AnthropicErrorSseFrame {
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }
}

#[must_use]
pub fn anthropic_error_sse_frame(status_code: u16, raw_message: &str) -> AnthropicErrorSseFrame {
    let frame = AnthropicErrorSseFrame::new(
        status_code,
        super::transform_messages_error(status_code, raw_message, None),
    );
    frame
}
