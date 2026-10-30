use betterlite_error::error::metadata::ErrorMetadata;
use serde::{Deserialize, Serialize};
use serde_json::value::{RawValue, to_raw_value};

use super::AnthropicExceptionMapping;
use super::decode::{WireDecodeError, WireDecodeLimits, bounded_json};
use super::fields::Fields;

use super::wire::{
    AnthropicErrorDetail, AnthropicErrorResponse, DetailRecord, ErrorField, ErrorRecord,
    Missing as _, ObjectField, ProviderMessage, RawField, TextField,
};

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

impl AnthropicExceptionMapping {
    #[must_use]
    pub const fn get_error_type(status: u16) -> AnthropicErrorType {
        match status {
            400 => AnthropicErrorType::InvalidRequestError,
            401 => AnthropicErrorType::AuthenticationError,
            403 => AnthropicErrorType::PermissionError,
            404 => AnthropicErrorType::NotFoundError,
            413 => AnthropicErrorType::RequestTooLarge,
            429 => AnthropicErrorType::RateLimitError,
            529 => AnthropicErrorType::OverloadedError,
            _ => AnthropicErrorType::ApiError,
        }
    }

    pub fn create_error_response(
        status: u16,
        message: TextField,
        request_id: Option<&str>,
    ) -> Result<ErrorRecord, WireDecodeError> {
        let error_type = to_raw_value(&Self::get_error_type(status))?;
        let detail = AnthropicErrorDetail {
            r#type: TextField::Other(error_type),
            message,
            provider_specific_fields: ObjectField::Missing,
            litellm_call_id: TextField::Missing,
        };
        let response = AnthropicErrorResponse {
            r#type: TextField::Known("error".to_owned()),
            error: ErrorField::Known(Box::new(DetailRecord::from_head(detail)?)),
            request_id: request_id
                .filter(|value| !value.is_empty())
                .map_or(TextField::Missing, |value| {
                    TextField::Known(value.to_owned())
                }),
        };
        ErrorRecord::from_head(response).map_err(WireDecodeError::Json)
    }

    pub fn transform_to_anthropic_error(
        status: u16,
        raw_message: &str,
        request_id: Option<&str>,
        limits: WireDecodeLimits,
    ) -> Result<ErrorRecord, WireDecodeError> {
        let parsed = match bounded_json(raw_message.as_bytes(), limits) {
            Ok(raw) => Some(raw),
            Err(WireDecodeError::Json(_)) => None,
            Err(error) => return Err(error),
        };
        let Some(raw) = parsed else {
            return Self::create_error_response(
                status,
                TextField::Known(raw_message.to_owned()),
                request_id,
            );
        };
        if let Ok(mut response) = serde_json::from_str::<ErrorRecord>(raw.get())
            && matches!(&response.head.r#type, TextField::Known(kind) if kind == "error")
            && matches!(&response.head.error, ErrorField::Known(error) if !error.head.r#type.is_missing() && !error.head.message.is_missing())
        {
            if let Some(identifier) = request_id.filter(|value| !value.is_empty())
                && response.head.request_id.is_missing()
            {
                response.head.request_id = TextField::Known(identifier.to_owned());
                response
                    .fields
                    .insert("request_id".to_owned(), to_raw_value(identifier)?);
            }
            return Ok(response);
        }
        if let Ok(fields) = serde_json::from_str::<Fields>(raw.get()) {
            let message = extracted_message(&fields, raw_message)?;
            return Self::create_error_response(status, message, request_id);
        }
        Self::create_error_response(status, TextField::Known(raw_message.to_owned()), request_id)
    }
}

fn extracted_message(fields: &Fields, raw: &str) -> Result<TextField, WireDecodeError> {
    // Python parses into a dict first, so duplicate provider keys have already resolved to their last value.
    let json = serde_json::to_string(fields)?;
    let provider: ProviderMessage = serde_json::from_str(&json)?;
    let chosen = match provider.detail {
        ObjectField::Known(mut detail) => detail.swap_remove("message"),
        ObjectField::Missing | ObjectField::Other(_) => [provider.aws, provider.message]
            .into_iter()
            .find_map(|field| match field {
                RawField::Known(value) if truthy(&value) => Some(value),
                RawField::Known(_) | RawField::Missing | RawField::Other(_) => None,
            }),
    };
    match chosen {
        Some(value) => Ok(serde_json::from_str(value.get())?),
        None => Ok(TextField::Known(raw.to_owned())),
    }
}

fn truthy(raw: &RawValue) -> bool {
    let text = raw.get();
    match text.as_bytes().first() {
        Some(b'"') => text != "\"\"",
        Some(b'{' | b'[') => text
            .get(1..text.len().saturating_sub(1))
            .is_some_and(|inner| !inner.trim().is_empty()),
        Some(b'n' | b'f') => false,
        Some(b'-' | b'0'..=b'9') => text.split(['e', 'E']).next().is_some_and(|mantissa| {
            mantissa
                .chars()
                .any(|character| !matches!(character, '-' | '.' | '0'))
        }),
        Some(_) | None => true,
    }
}

impl From<&ErrorMetadata> for AnthropicErrorType {
    fn from(error: &ErrorMetadata) -> Self {
        AnthropicExceptionMapping::get_error_type(error.status_code)
    }
}
