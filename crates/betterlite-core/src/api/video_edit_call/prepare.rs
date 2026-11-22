use std::str::FromStr as _;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_environment_variables};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::openai_video_identifiers::ids::decode_video_id_with_provider;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use super::error::{VideoEditCallError, VideoEditFailure, failure};
use super::mock::mock_response;
use super::request::Ready;
use super::{
    ASYNC_CALL, CALL_TYPE, DEFAULT_PROVIDER, DEFAULT_TIMEOUT, LITELLM_CALL_ID, LOGGING_OBJECT,
    USER, VideoEditCall, VideoEditConfigLookup, VideoEditDeps,
};
use crate::api::vector_store_http_write::service::SuppliedClient;
use crate::api::video_edit_transport::calls::VideoEditHandlers;
use crate::api::video_edit_transport::config::{BaseVideoConfig, Params};
use crate::api::video_edit_transport::error::VideoClientError;
use crate::api::video_records::VideoObject;
use crate::api::video_selection::get_provider_video_config;

pub(crate) enum Prepared<'run, Client> {
    Mock(VideoObject),
    Call(Ready<'run, Client>),
}

// LiteLLM's video_edit body, steps 1 to 11 in order
pub(crate) fn prepare<'run, Configs, Handler>(
    deps: &'run VideoEditDeps<'run, Configs, Handler>,
    call: &VideoEditCall<'_>,
) -> Result<Prepared<'run, Handler::Client>, VideoEditFailure>
where
    Configs: VideoEditConfigLookup + ?Sized,
    Handler: VideoEditHandlers + ?Sized,
{
    require_logging_object(call, deps.redactor)?;
    let is_async = matches!(call.kwargs.get(ASYNC_CALL), Some(Json::Bool(true)));
    if let Some(object) = mock_from(call, deps.redactor)? {
        return Ok(Prepared::Mock(object));
    }
    let provider = provider_name(call.custom_llm_provider, call.video_id);
    let litellm_params = generic_params(call.kwargs);
    let config = config_for(deps.configs, &provider)
        .map_err(|error| failure(error, &provider, deps.redactor))?;
    log_call(deps.logging, call, &provider, deps.redactor)?;
    Ok(Prepared::Call(Ready {
        provider,
        config,
        prompt: call.prompt.to_owned(),
        video_id: call.video_id.to_owned(),
        video: call.video.cloned(),
        extra_headers: call.extra_headers.cloned(),
        extra_body: call.extra_body.cloned(),
        litellm_params,
        timeout: timeout(call.timeout, deps.request_timeout),
        is_async,
        client: deps.client.as_ref().map(|client| SuppliedClient {
            kind: client.kind,
            transport: client.transport,
        }),
    }))
}

// step 1: the logging object arrives in the loose keywords, and a missing one fails before anything runs
fn require_logging_object(
    call: &VideoEditCall<'_>,
    redactor: &Redactor,
) -> Result<(), VideoEditFailure> {
    if matches!(call.kwargs.get(LOGGING_OBJECT), None | Some(Json::Null)) {
        return Err(failure(
            VideoEditCallError::MissingLoggingObject,
            call.custom_llm_provider.unwrap_or_default(),
            redactor,
        ));
    }
    Ok(())
}

// step 4: the mock is read, not removed, and answers before the provider lookup
fn mock_from(
    call: &VideoEditCall<'_>,
    redactor: &Redactor,
) -> Result<Option<VideoObject>, VideoEditFailure> {
    let given = call.custom_llm_provider.unwrap_or_default();
    mock_response(call.kwargs)
        .map_err(|error| failure(VideoEditCallError::Mock(error), given, redactor))
}

// step 5: the caller's provider, else the one the video id names, with "openai" for neither
fn provider_name(explicit: Option<&str>, video_id: &str) -> String {
    if let Some(provider) = explicit {
        return provider.to_owned();
    }
    let provider = decode_video_id_with_provider(video_id)
        .custom_llm_provider
        .unwrap_or_default();
    if provider.is_empty() {
        DEFAULT_PROVIDER.to_owned()
    } else {
        provider
    }
}

// step 6: GenericLiteLLMParams from the keywords that are left after the two pops
fn generic_params(kwargs: &IndexMap<String, Json>) -> Params {
    let mut params = kwargs.clone();
    params.shift_remove(LOGGING_OBJECT);
    params.shift_remove(ASYNC_CALL);
    params
}

// step 7: litellm.LlmProviders(custom_llm_provider), then ProviderConfigManager.get_provider_video_config(model=None)
fn config_for<'run, Configs>(
    configs: &'run Configs,
    provider: &str,
) -> Result<&'run dyn BaseVideoConfig, VideoEditCallError>
where
    Configs: VideoEditConfigLookup + ?Sized,
{
    let Some(id) = ProviderId::from_str(provider).ok() else {
        let unknown = VideoClientError::UnknownProvider {
            provider: provider.to_owned(),
        };
        return Err(VideoEditCallError::UnknownProvider(unknown));
    };
    let selected = get_provider_video_config(None, id)?;
    let Some(tag) = selected else {
        return Err(VideoEditCallError::ProviderConfigMissing {
            provider: provider.to_owned(),
        });
    };
    configs
        .video_config(&tag)
        .ok_or_else(|| VideoEditCallError::ProviderConfigMissing {
            provider: provider.to_owned(),
        })
}

struct LoggingUpdate {
    litellm_params: IndexMap<String, Json>,
    call: CallUpdate,
}

// steps 9 and 10: update_environment_variables with model "", the request params and the call id first, then call_type
fn log_call(
    logging: &Mutex<LoggingState>,
    call: &VideoEditCall<'_>,
    provider: &str,
    redactor: &Redactor,
) -> Result<(), VideoEditFailure> {
    let update = logging_update(call, provider, redactor)?;
    let mut state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    update_environment_variables(&mut state, &update.litellm_params, update.call)
        .map_err(|error| failure(VideoEditCallError::Logging(error), provider, redactor))?;
    CALL_TYPE.clone_into(&mut state.call_type);
    drop(state);
    Ok(())
}

fn logging_update(
    call: &VideoEditCall<'_>,
    provider: &str,
    redactor: &Redactor,
) -> Result<LoggingUpdate, VideoEditFailure> {
    let optional_params = IndexMap::from([
        ("video_id".to_owned(), Json::Text(call.video_id.to_owned())),
        ("prompt".to_owned(), Json::Text(call.prompt.to_owned())),
    ]);
    let mut litellm_params = IndexMap::from([(
        LITELLM_CALL_ID.to_owned(),
        call.kwargs
            .get(LITELLM_CALL_ID)
            .cloned()
            .unwrap_or(Json::Null),
    )]);
    for (name, value) in &optional_params {
        litellm_params.insert(name.clone(), value.clone());
    }
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    let recorded_user = user(call.kwargs)
        .map_err(|error| failure(VideoEditCallError::Logging(error), provider, redactor))?;
    Ok(LoggingUpdate {
        litellm_params,
        call: CallUpdate {
            model: Some(String::new()),
            user: recorded_user,
            optional_params,
            additional_params,
        },
    })
}

// user=kwargs.get("user") in the logging update, refused when it is not a string
fn user(kwargs: &IndexMap<String, Json>) -> Result<Option<String>, LoggingStateError> {
    match kwargs.get(USER) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(LoggingStateError::FieldType { field: USER })
        }
    }
}

// Python's `timeout or DEFAULT_REQUEST_TIMEOUT`: the declared default is 600 seconds, and a zero falls to B198's timeout
const fn timeout(timeout: Option<Duration>, request_timeout: Duration) -> Duration {
    match timeout {
        None => DEFAULT_TIMEOUT,
        Some(value) if value.is_zero() => request_timeout,
        Some(value) => value,
    }
}
