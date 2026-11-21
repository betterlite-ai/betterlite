use betterlite_prompts::arguments::json::{Json, Object};
use serde::de::DeserializeOwned;

use crate::api::genai::chat_call::CompletionExecutor;
use crate::api::genai_http::config::GenerateContentHttpConfig;
use crate::api::genai_http::service::ClientSource;
use crate::genai_setup::params::RequestFieldError;

use super::error::{GenerateContentError, GenerateContentFailure, failure};
use super::{GenerateContentDeps, GenerateContentRequest};

// docs/compatibility.md: a value outside its declared type is an error that names the field
pub(super) fn decode<T>(
    value: &Json,
    field: &'static str,
    expected: &'static str,
) -> Result<T, RequestFieldError>
where
    T: DeserializeOwned,
{
    let encoded = serde_json::to_value(value);
    if let Ok(encoded) = encoded
        && let Ok(typed) = serde_json::from_value(encoded)
    {
        return Ok(typed);
    }
    Err(RequestFieldError { field, expected })
}

// kwargs.get("systemInstruction") or kwargs.get("system_instruction"): Python picks the second only when the first is falsy
pub(super) fn first_non_empty<'object>(
    kwargs: &'object Object,
    names: [&str; 2],
) -> Option<&'object Json> {
    let [first, second] = names;
    let value = kwargs.get(first).filter(|value| value.is_truthy());
    value.or_else(|| kwargs.get(second).filter(|value| value.is_truthy()))
}

// GenerateContentToCompletionHandler forwards these kwargs members to the completion call as they are
pub(super) fn object_field<T>(
    kwargs: &Object,
    field: &'static str,
) -> Result<Option<T>, RequestFieldError>
where
    T: DeserializeOwned,
{
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => decode(value, field, "an object").map(Some),
    }
}

// an entry step that reads a kwargs field answers with the local exception_type, before any provider runs
pub(super) fn field_failure<C, S, Executor>(
    error: RequestFieldError,
    request: &GenerateContentRequest<'_>,
    provider: Option<&str>,
    deps: &GenerateContentDeps<'_, C, S, Executor>,
) -> GenerateContentFailure<C::Error, Executor::Error>
where
    C: GenerateContentHttpConfig,
    S: ClientSource,
    Executor: CompletionExecutor,
{
    failure(
        GenerateContentError::Fields(error),
        request.model,
        provider,
        deps.redactor,
    )
}
