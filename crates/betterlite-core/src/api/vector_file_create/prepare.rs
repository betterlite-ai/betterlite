use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_file_contracts::records::{Attributes, FileParams};
use indexmap::IndexMap;
use secrecy::SecretString;

use super::error::FileCallError;
use crate::api::vector_file_http_create_list::config::FilesParams;

// GenericLiteLLMParams(**kwargs) read through the three fields the handler calls use
pub(crate) fn files_params(kwargs: &IndexMap<String, Json>) -> Result<FilesParams, FileCallError> {
    let api_base = match kwargs.get("api_base") {
        None | Some(Json::Null) => None,
        Some(Json::Text(value)) => Some(value.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(FileCallError::Field {
                field: "api_base",
                expected: "a string",
            });
        }
    };
    let api_key = match kwargs.get("api_key") {
        None | Some(Json::Null) => None,
        Some(Json::Text(value)) => Some(value.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(FileCallError::Field {
                field: "api_key",
                expected: "a string",
            });
        }
    };
    let ssl_verify = match kwargs.get("ssl_verify") {
        None | Some(Json::Null) => None,
        Some(Json::Bool(value)) => Some(*value),
        Some(Json::Text(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(FileCallError::Field {
                field: "ssl_verify",
                expected: "a boolean",
            });
        }
    };
    Ok(FilesParams {
        api_base,
        api_key: api_key.map(SecretString::from),
        ssl_verify,
    })
}

pub(crate) fn create_locals(
    file_id: &str,
    attributes: Option<&Attributes>,
    chunking_strategy: Option<&IndexMap<String, Json>>,
) -> Result<FileParams, serde_json::Error> {
    let mut locals = FileParams::new();
    locals.insert(String::from("file_id"), serde_json::to_value(file_id)?);
    if let Some(values) = attributes {
        locals.insert(String::from("attributes"), serde_json::to_value(values)?);
    }
    if let Some(chunking) = chunking_strategy {
        locals.insert(
            String::from("chunking_strategy"),
            serde_json::to_value(chunking)?,
        );
    }
    Ok(locals)
}

pub(crate) fn list_locals(
    after: Option<&str>,
    before: Option<&str>,
    filter: Option<&str>,
    limit: Option<i64>,
    order: Option<&str>,
) -> Result<FileParams, serde_json::Error> {
    let mut locals = FileParams::new();
    for (name, value) in [("after", after), ("before", before), ("filter", filter)] {
        if let Some(entry) = value {
            locals.insert(name.to_owned(), serde_json::to_value(entry)?);
        }
    }
    if let Some(maximum) = limit {
        locals.insert(String::from("limit"), serde_json::to_value(maximum)?);
    }
    if let Some(direction) = order {
        locals.insert(String::from("order"), serde_json::to_value(direction)?);
    }
    Ok(locals)
}

pub(crate) fn log_call(
    logging: &Mutex<LoggingState>,
    kwargs: &IndexMap<String, Json>,
    vector_store_id: &str,
    filtered: &FileParams,
    provider: &str,
) -> Result<(), FileCallError> {
    let optional_params = optional_params_of(vector_store_id, filtered)?;
    let mut litellm_params = IndexMap::from([
        (
            String::from("vector_store_id"),
            Json::Text(vector_store_id.to_owned()),
        ),
        (
            String::from("litellm_call_id"),
            kwargs.get("litellm_call_id").cloned().unwrap_or(Json::Null),
        ),
    ]);
    litellm_params.extend(generic_params_dump(kwargs));
    let additional_params = IndexMap::from([(
        String::from("custom_llm_provider"),
        Json::Text(provider.to_owned()),
    )]);
    let mut state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    update_from_kwargs(
        &mut state,
        kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )
    .map_err(FileCallError::Logging)
}

fn optional_params_of(
    vector_store_id: &str,
    filtered: &FileParams,
) -> Result<IndexMap<String, Json>, FileCallError> {
    let mut optional_params = IndexMap::from([(
        String::from("vector_store_id"),
        Json::Text(vector_store_id.to_owned()),
    )]);
    for (name, value) in filtered {
        optional_params.insert(
            name.clone(),
            serde_json::from_value(value.clone()).map_err(FileCallError::Params)?,
        );
    }
    Ok(optional_params)
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

pub(crate) fn call_timeout(
    requested: Option<f64>,
    request_timeout_seconds: f64,
) -> Result<Duration, FileCallError> {
    let seconds = match requested {
        Some(seconds) if seconds != 0.0_f64 => seconds,
        Some(_) | None => request_timeout_seconds,
    };
    Duration::try_from_secs_f64(seconds)
        .map_err(|source| FileCallError::Timeout { seconds, source })
}
