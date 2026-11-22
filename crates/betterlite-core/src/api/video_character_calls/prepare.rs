use std::time::Duration;

use betterlite_http::interactions::base::Params;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use indexmap::IndexMap;

use super::error::CharacterCallError;
use super::logging::{LoggedCall, log_call};
use super::mock::mock_response;
use super::params;
use super::{CREATE_CALL_TYPE, CreateCharacterCall, GET_CALL_TYPE, GetCharacterCall};
use crate::api::video_records::CharacterObject;
use crate::api::video_selection::get_provider_video_config;

// a mock character, or everything a handler call needs
pub(crate) enum Prepared {
    Mock(CharacterObject),
    Call(ReadyCall),
}

pub(crate) struct ReadyCall {
    pub(crate) litellm_params: Params,
    pub(crate) timeout: Duration,
}

// which operation's wording and call type an unresolvable provider uses
#[derive(Clone, Copy)]
enum CharacterOperation {
    Create,
    Get,
}

impl CharacterOperation {
    const fn unsupported(self, provider: String) -> CharacterCallError {
        match self {
            Self::Create => CharacterCallError::CreateUnsupported { provider },
            Self::Get => CharacterCallError::GetUnsupported { provider },
        }
    }

    const fn call_type(self) -> &'static str {
        match self {
            Self::Create => CREATE_CALL_TYPE,
            Self::Get => GET_CALL_TYPE,
        }
    }
}

// the one primary argument and the keywords both operations read
struct CallParts<'call> {
    operation: CharacterOperation,
    provider: &'call str,
    field: &'call str,
    value: &'call str,
    litellm_call_id: Option<&'call str>,
    extra: &'call IndexMap<String, Json>,
    timeout: Option<f64>,
}

// create_character's steps 1 to 8, before the handler call
pub(crate) fn prepare_create(
    logging: &mut LoggingState,
    request_timeout_seconds: f64,
    call: &CreateCharacterCall,
) -> Result<Prepared, CharacterCallError> {
    if let Some(character) = mock_response(call.mock_response.as_ref(), "a character object")? {
        return Ok(Prepared::Mock(character));
    }
    let parts = CallParts {
        operation: CharacterOperation::Create,
        provider: &call.custom_llm_provider,
        field: "name",
        value: &call.name,
        litellm_call_id: call.litellm_call_id.as_deref(),
        extra: &call.extra,
        timeout: call.timeout,
    };
    ready(logging, request_timeout_seconds, &parts).map(Prepared::Call)
}

// get_character's steps 1 to 8, before the handler call
pub(crate) fn prepare_get(
    logging: &mut LoggingState,
    request_timeout_seconds: f64,
    call: &GetCharacterCall,
) -> Result<Prepared, CharacterCallError> {
    if let Some(character) = mock_response(call.mock_response.as_ref(), "a character object")? {
        return Ok(Prepared::Mock(character));
    }
    let parts = CallParts {
        operation: CharacterOperation::Get,
        provider: &call.custom_llm_provider,
        field: "character_id",
        value: &call.character_id,
        litellm_call_id: call.litellm_call_id.as_deref(),
        extra: &call.extra,
        timeout: call.timeout,
    };
    ready(logging, request_timeout_seconds, &parts).map(Prepared::Call)
}

// LiteLLM's LlmProviders(provider), B3332's selector, the logging update, then `timeout or DEFAULT_REQUEST_TIMEOUT`
fn ready(
    logging: &mut LoggingState,
    request_timeout_seconds: f64,
    parts: &CallParts<'_>,
) -> Result<ReadyCall, CharacterCallError> {
    let litellm_params = params::request_params(parts.extra, parts.litellm_call_id);
    let Ok(provider) = parts.provider.parse::<ProviderId>() else {
        return Err(CharacterCallError::UnknownProvider {
            provider: parts.provider.to_owned(),
        });
    };
    if get_provider_video_config(None, provider)?.is_none() {
        return Err(parts.operation.unsupported(parts.provider.to_owned()));
    }
    let user = params::user(parts.extra)?;
    log_call(
        logging,
        &LoggedCall {
            field: parts.field,
            value: parts.value,
            call_id: parts.litellm_call_id,
            provider: parts.provider,
            call_type: parts.operation.call_type(),
            user,
        },
    )?;
    let timeout = resolved_timeout(parts.timeout, request_timeout_seconds)?;
    Ok(ReadyCall {
        litellm_params,
        timeout,
    })
}

// `timeout or DEFAULT_REQUEST_TIMEOUT`: absent, null and zero fall back to B198's REQUEST_TIMEOUT seconds
fn resolved_timeout(
    timeout: Option<f64>,
    request_timeout_seconds: f64,
) -> Result<Duration, CharacterCallError> {
    let seconds = timeout
        .filter(|seconds| *seconds != 0.0)
        .unwrap_or(request_timeout_seconds);
    Duration::try_from_secs_f64(seconds)
        .map_err(|source| CharacterCallError::Timeout { seconds, source })
}
