use std::str::FromStr as _;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_context::resource_config_selection::vector_stores::{
    VectorStoreFilesConfigTag, select_store_files_config,
};
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::vector_file_contracts::records::{
    Attributes, VectorStoreFileUpdateRequest,
};
use indexmap::IndexMap;
use secrecy::SecretString;

use super::VectorFileWriteDeps;
use super::error::{VectorFileWriteError, VectorFileWriteFailure, failure};
use super::service::flag;
use crate::api::vector_file_http_write::config::WriteParams;
use crate::api::vector_store_http_write::service::Operation;

pub(crate) struct Target {
    pub(crate) id: ProviderId,
    pub(crate) config: VectorStoreFilesConfigTag,
}

// B174's enum conversion, then B010's configuration for the provider the call resolved to
pub(crate) fn target(
    operation: Operation,
    provider: &str,
    deps: VectorFileWriteDeps<'_>,
) -> Result<Target, VectorFileWriteFailure> {
    let parsed =
        ProviderId::from_str(provider).map_err(|_invalid| VectorFileWriteError::InvalidProvider {
            provider: provider.to_owned(),
        });
    let id = parsed.map_err(|error| failure(error, provider, deps.redactor))?;
    let Some(config) = select_store_files_config(id) else {
        let error = VectorFileWriteError::Unsupported {
            operation,
            provider: provider.to_owned(),
        };
        return Err(failure(error, provider, deps.redactor));
    };
    Ok(Target { id, config })
}

// GenericLiteLLMParams over the merged kwargs: pydantic reads these three fields off the dump the handler is given
pub(crate) fn write_params(
    kwargs: &IndexMap<String, Json>,
) -> Result<WriteParams, VectorFileWriteError> {
    let api_base = optional_text(kwargs, "api_base")?;
    let api_key = optional_text(kwargs, "api_key")?;
    let ssl_verify = match kwargs.get("ssl_verify") {
        None | Some(Json::Null) => None,
        Some(Json::Bool(flag)) => Some(*flag),
        Some(_other) => {
            return Err(VectorFileWriteError::FieldType {
                field: "ssl_verify",
                expected: "a boolean",
            });
        }
    };
    Ok(WriteParams {
        api_base,
        api_key: api_key.map(SecretString::from),
        ssl_verify,
    })
}

fn optional_text(
    kwargs: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<String>, VectorFileWriteError> {
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_other) => Err(VectorFileWriteError::FieldType {
            field,
            expected: "a string",
        }),
    }
}

// get_update_request_params keeps attributes and drops a null, then LiteLLM restores the argument it was handed
pub(crate) fn update_values(
    attributes: Option<&Attributes>,
) -> Result<VectorStoreFileUpdateRequest, VectorFileWriteError> {
    let mut request = VectorStoreFileUpdateRequest::default();
    let Some(values) = attributes else {
        // the record keeps the restored null beside its optional field, where a passthrough value carries it
        let null = serde_json::to_value(()).map_err(VectorFileWriteError::Params)?;
        request.extra.insert(String::from("attributes"), null);
        return Ok(request);
    };
    request.attributes = Some(values.clone());
    Ok(request)
}

// the two ids every file write logs, whichever handler runs
fn ids(store_id: &str, file_id: &str) -> IndexMap<String, Json> {
    IndexMap::from([
        (
            String::from("vector_store_id"),
            Json::Text(store_id.to_owned()),
        ),
        (String::from("file_id"), Json::Text(file_id.to_owned())),
    ])
}

fn attributes_value(attributes: Option<&Attributes>) -> Result<Json, VectorFileWriteError> {
    let Some(values) = attributes else {
        return Ok(Json::Null);
    };
    let encoded = serde_json::to_value(values).map_err(VectorFileWriteError::Params)?;
    serde_json::from_value(encoded).map_err(VectorFileWriteError::Params)
}

// One file write's log call: update_from_kwargs(kwargs, model=None, optional_params={ids, attributes for an update}, ...)
#[derive(Clone, Copy)]
pub(crate) struct LogWrite<'write> {
    pub(crate) logging: &'write Mutex<LoggingState>,
    pub(crate) operation: Operation,
    pub(crate) store_id: &'write str,
    pub(crate) file_id: &'write str,
    pub(crate) attributes: Option<&'write Attributes>,
    pub(crate) kwargs: &'write IndexMap<String, Json>,
    pub(crate) provider: &'write str,
}

pub(crate) fn log_write(entry: LogWrite<'_>) -> Result<(), VectorFileWriteError> {
    // the async flag is popped before LiteLLM logs, so it stays out of the forwarded kwargs
    let mut forwarded = entry.kwargs.clone();
    forwarded.shift_remove(flag(entry.operation));
    let call_id = forwarded
        .get("litellm_call_id")
        .cloned()
        .unwrap_or(Json::Null);
    let mut litellm_params = IndexMap::from([
        (
            String::from("vector_store_id"),
            Json::Text(entry.store_id.to_owned()),
        ),
        (String::from("litellm_call_id"), call_id),
    ]);
    litellm_params.extend(generic_params_dump(&forwarded));
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        String::from("custom_llm_provider"),
        Json::Text(entry.provider.to_owned()),
    );
    let mut state = entry.logging.lock().unwrap_or_else(PoisonError::into_inner);
    update_from_kwargs(
        &mut state,
        &forwarded,
        Some(&litellm_params),
        CallUpdate {
            model: None,
            user: None,
            optional_params: logged_params(&entry)?,
            additional_params,
        },
    )
    .map_err(VectorFileWriteError::Logging)
}

// an update logs the argument it was handed; a delete keeps both ids alone
fn logged_params(entry: &LogWrite<'_>) -> Result<IndexMap<String, Json>, VectorFileWriteError> {
    let mut params = ids(entry.store_id, entry.file_id);
    if entry.operation == Operation::Update {
        params.insert(
            String::from("attributes"),
            attributes_value(entry.attributes)?,
        );
    }
    Ok(params)
}

// GenericLiteLLMParams.model_dump(exclude_none=True): the five fields defaulting to False, then every non-null kwarg
fn generic_params_dump(kwargs: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let mut dump: IndexMap<String, Json> = [
        "allow_client_keepalive_override",
        "merge_reasoning_content_in_choices",
        "use_in_pass_through",
        "use_litellm_proxy",
        "use_xai_oauth",
    ]
    .into_iter()
    .map(|name| (name.to_owned(), Json::Bool(false)))
    .collect();
    dump.extend(
        kwargs
            .iter()
            .filter(|(_, value)| !matches!(value, Json::Null))
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    dump
}

// `timeout or request_timeout`: an explicit zero is falsy, and the module constant is never absent
pub(crate) fn call_timeout(
    requested: Option<f64>,
    request_timeout_seconds: f64,
) -> Result<Duration, VectorFileWriteError> {
    let seconds = match requested {
        Some(seconds) if seconds != 0.0_f64 => seconds,
        Some(_) | None => request_timeout_seconds,
    };
    Duration::try_from_secs_f64(seconds)
        .map_err(|source| VectorFileWriteError::Timeout { seconds, source })
}
