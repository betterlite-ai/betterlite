use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::api::video_remix_list_transport::records::Params;

// LiteLLM's mock short circuit: a string is parsed as JSON first, then the typed answer is built
pub(crate) fn mock_answer<T>(
    mock: Option<&Json>,
    expected: &'static str,
) -> Result<Option<T>, MockDecodeError>
where
    T: DeserializeOwned,
{
    let value = match mock {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Text(text)) => {
            serde_json::from_str::<Json>(text).map_err(|source| MockDecodeError::Json { source })?
        }
        Some(value) => value.clone(),
    };
    let encoded = serde_json::to_value(&value)
        .map_err(|source| MockDecodeError::Model { expected, source })?;
    serde_json::from_value(encoded)
        .map(Some)
        .map_err(|source| MockDecodeError::Model { expected, source })
}

#[derive(Debug, Error)]
pub enum MockDecodeError {
    #[error("the mock response is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the mock response does not match {expected}")]
    Model {
        expected: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

// GenericLiteLLMParams(**kwargs): the untyped keywords plus the typed credentials the transport reads
pub(crate) fn transport_params(
    extra: &IndexMap<String, Json>,
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    api_version: Option<&str>,
) -> Params {
    let mut params = extra.clone();
    if let Some(key) = api_key {
        params.insert(
            "api_key".to_owned(),
            Json::Text(key.expose_secret().to_owned()),
        );
    }
    if let Some(base) = api_base {
        params.insert("api_base".to_owned(), Json::Text(base.to_owned()));
    }
    if let Some(version) = api_version {
        params.insert("api_version".to_owned(), Json::Text(version.to_owned()));
    }
    params
}

// the keywords update_from_kwargs sees: the logging object and the async flag are popped before it
pub(crate) fn call_kwargs(
    extra: &IndexMap<String, Json>,
    litellm_metadata: Option<&IndexMap<String, JsonValue>>,
    litellm_call_id: Option<&str>,
) -> Result<IndexMap<String, Json>, serde_json::Error> {
    let mut kwargs = extra.clone();
    kwargs.shift_remove("async_call");
    kwargs.shift_remove("litellm_logging_obj");
    if let Some(call_id) = litellm_call_id {
        kwargs.insert("litellm_call_id".to_owned(), Json::Text(call_id.to_owned()));
    }
    if let Some(metadata) = litellm_metadata {
        let encoded = serde_json::to_string(metadata)?;
        kwargs.insert(
            "litellm_metadata".to_owned(),
            serde_json::from_str(&encoded)?,
        );
    }
    Ok(kwargs)
}

pub(crate) struct LoggedCall<'call> {
    pub(crate) kwargs: &'call IndexMap<String, Json>,
    pub(crate) optional_params: &'call IndexMap<String, Json>,
    pub(crate) call_id: Option<&'call str>,
    pub(crate) provider: &'call str,
    pub(crate) user: Option<&'call str>,
    pub(crate) call_type: &'call str,
}

// update_from_kwargs(kwargs, model="", user=..., optional_params=params, litellm_params={"litellm_call_id", **params})
pub(crate) fn log_call(
    state: &mut LoggingState,
    call: &LoggedCall<'_>,
) -> Result<(), LoggingStateError> {
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        call.call_id
            .map_or(Json::Null, |call_id| Json::Text(call_id.to_owned())),
    );
    for (name, value) in call.optional_params {
        litellm_params.insert(name.clone(), value.clone());
    }
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(call.provider.to_owned()),
    );
    update_from_kwargs(
        state,
        call.kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(String::new()),
            user: call.user.map(str::to_owned),
            optional_params: call.optional_params.clone(),
            additional_params,
        },
    )?;
    call.call_type.clone_into(&mut state.call_type);
    Ok(())
}
