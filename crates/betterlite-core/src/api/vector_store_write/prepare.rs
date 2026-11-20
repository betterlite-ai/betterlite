use std::str::FromStr as _;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_context::resource_config_selection::vector_stores::{
    VectorStoresConfigTag, select_vector_stores_config,
};
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use indexmap::IndexMap;
use serde::Serialize;

use super::error::{VectorStoreWriteError, VectorStoreWriteFailure, failure};
use super::service::{DELETE_FLAG, Operation, UPDATE_FLAG};
use super::{DEFAULT_PROVIDER, TransportRequest, VectorStoreWriteDeps, VectorStoreWriteRequest};
use crate::api::vector_store_http_write::params::UpdateParams;

// VectorStoreCreateOptionalRequestParams: the only keys an update may carry, named or through kwargs
const CREATE_FIELDS: [&str; 5] = [
    "name",
    "file_ids",
    "expires_after",
    "chunking_strategy",
    "metadata",
];

pub(crate) struct Target {
    pub(crate) name: String,
    pub(crate) id: ProviderId,
    pub(crate) config: VectorStoresConfigTag,
}

// the provider the call resolves to and the vector stores configuration B010 selects for it
pub(crate) fn target(
    operation: Operation,
    transport: &TransportRequest<'_>,
    deps: VectorStoreWriteDeps<'_>,
) -> Result<Target, VectorStoreWriteFailure> {
    let name = transport.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
    let declared = resolve_provider(name, deps.resolution)
        .map_err(|error| failure(error, name, deps.redactor))?;
    let Some(config) = select_vector_stores_config(declared.id, declared.api_type.as_deref())
    else {
        let error = VectorStoreWriteError::Unsupported {
            operation,
            provider: declared.name.clone(),
        };
        return Err(failure(error, &declared.name, deps.redactor));
    };
    Ok(Target {
        name: declared.name,
        id: declared.id,
        config,
    })
}

struct Declared {
    name: String,
    id: ProviderId,
    api_type: Option<String>,
}

// A slash makes the prefix the provider and the rest the api_type; a bare value is the provider name
fn resolve_provider(
    declaration: &str,
    resolution: &ResolutionContext<'_>,
) -> Result<Declared, VectorStoreWriteError> {
    if !declaration.contains('/') {
        let id = ProviderId::from_str(declaration).map_err(|_invalid| {
            VectorStoreWriteError::InvalidProvider {
                provider: declaration.to_owned(),
            }
        })?;
        return Ok(Declared {
            name: declaration.to_owned(),
            id,
            api_type: None,
        });
    }
    let request = ResolutionRequest {
        model: declaration,
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    let resolved = resolve(&request, resolution)?;
    let name = resolved.provider.slug().to_owned();
    let id =
        ProviderId::from_str(&name).map_err(|_invalid| VectorStoreWriteError::InvalidProvider {
            provider: name.clone(),
        })?;
    Ok(Declared {
        name,
        id,
        api_type: Some(resolved.model),
    })
}

// local_vars.update(kwargs), then get_requested_vector_store_create_optional_param's non-None filter
pub(crate) fn update_options(
    request: &VectorStoreWriteRequest<'_>,
    deps: VectorStoreWriteDeps<'_>,
    provider: &str,
) -> Result<UpdateParams, VectorStoreWriteFailure> {
    let mut candidates = IndexMap::new();
    insert_candidate(&mut candidates, "name", request.name)
        .map_err(VectorStoreWriteError::Params)
        .map_err(|error| failure(error, provider, deps.redactor))?;
    insert_candidate(&mut candidates, "expires_after", request.expires_after)
        .map_err(VectorStoreWriteError::Params)
        .map_err(|error| failure(error, provider, deps.redactor))?;
    insert_candidate(&mut candidates, "metadata", request.metadata)
        .map_err(VectorStoreWriteError::Params)
        .map_err(|error| failure(error, provider, deps.redactor))?;
    for field in CREATE_FIELDS {
        if let Some(value) = request.transport.kwargs.get(field) {
            candidates.insert(field.to_owned(), value.clone());
        }
    }
    candidates.retain(|_field, value| !matches!(value, Json::Null));
    log_update(deps.logging, request, &candidates, provider)
        .map_err(VectorStoreWriteError::Logging)
        .map_err(|error| failure(error, provider, deps.redactor))?;
    serde_json::to_value(candidates)
        .and_then(serde_json::from_value::<UpdateParams>)
        .map_err(VectorStoreWriteError::Params)
        .map_err(|error| failure(error, provider, deps.redactor))
}

// delete logs the id alone; the parameters B2402 sends are the empty create-parameter filter output
pub(crate) fn log_delete(
    request: &VectorStoreWriteRequest<'_>,
    deps: VectorStoreWriteDeps<'_>,
    provider: &str,
) -> Result<UpdateParams, VectorStoreWriteFailure> {
    let mut optional_params = IndexMap::new();
    optional_params.insert(
        "vector_store_id".to_owned(),
        Json::Text(request.transport.vector_store_id.to_owned()),
    );
    log_call(
        deps.logging,
        request.transport.kwargs,
        DELETE_FLAG,
        optional_params,
        provider,
    )
    .map_err(|error| {
        failure(
            VectorStoreWriteError::Logging(error),
            provider,
            deps.redactor,
        )
    })?;
    Ok(UpdateParams::default())
}

// `timeout or request_timeout`: an explicit zero is falsy, and the module constant is never absent
pub(crate) fn call_timeout(
    requested: Option<f64>,
    request_timeout_seconds: f64,
) -> Result<Duration, VectorStoreWriteError> {
    let seconds = match requested {
        Some(seconds) if seconds != 0.0_f64 => seconds,
        Some(_) | None => request_timeout_seconds,
    };
    Duration::try_from_secs_f64(seconds)
        .map_err(|source| VectorStoreWriteError::Timeout { seconds, source })
}

fn insert_candidate<T>(
    candidates: &mut IndexMap<String, Json>,
    field: &str,
    value: Option<&T>,
) -> Result<(), serde_json::Error>
where
    T: Serialize + ?Sized,
{
    let Some(value) = value else {
        return Ok(());
    };
    let encoded = serde_json::to_value(value)?;
    candidates.insert(field.to_owned(), serde_json::from_value(encoded)?);
    Ok(())
}

// update_from_kwargs(kwargs, model=None, optional_params={"vector_store_id": ..., "name": name, **filtered}, ...)
fn log_update(
    logging: &Mutex<LoggingState>,
    request: &VectorStoreWriteRequest<'_>,
    filtered: &IndexMap<String, Json>,
    provider: &str,
) -> Result<(), LoggingStateError> {
    let mut optional_params = IndexMap::new();
    optional_params.insert(
        "vector_store_id".to_owned(),
        Json::Text(request.transport.vector_store_id.to_owned()),
    );
    optional_params.insert(
        "name".to_owned(),
        request
            .name
            .map_or(Json::Null, |name| Json::Text(name.to_owned())),
    );
    for (field, value) in filtered {
        optional_params.insert(field.clone(), value.clone());
    }
    log_call(
        logging,
        request.transport.kwargs,
        UPDATE_FLAG,
        optional_params,
        provider,
    )
}

// the async flag is popped before LiteLLM logs, so it is read and then kept out of the forwarded kwargs
fn log_call(
    logging: &Mutex<LoggingState>,
    kwargs: &IndexMap<String, Json>,
    flag: &str,
    optional_params: IndexMap<String, Json>,
    provider: &str,
) -> Result<(), LoggingStateError> {
    let mut forwarded = kwargs.clone();
    forwarded.shift_remove(flag);
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        forwarded
            .get("litellm_call_id")
            .cloned()
            .unwrap_or(Json::Null),
    );
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    let mut state = logging.lock().unwrap_or_else(PoisonError::into_inner);
    update_from_kwargs(
        &mut state,
        &forwarded,
        Some(&litellm_params),
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )
}
