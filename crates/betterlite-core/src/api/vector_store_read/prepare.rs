use std::sync::PoisonError;
use std::time::Duration;

use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use indexmap::IndexMap;
use serde_json::Number;

use super::DEFAULT_PROVIDER;
use super::error::{VectorStoreReadError, VectorStoreReadFailure, failure};
use super::service::{LIST_FLAG, ListQuery, Operation, RETRIEVE_FLAG, ReadConfig};
use super::{
    Content, TransportRequest, VectorStoreListRequest, VectorStoreReadDeps, VectorStoreReadRequest,
};
use crate::api::vector_store_search::provider::resolve_provider;

pub(crate) struct Target<'deps> {
    pub(crate) operation: Operation,
    // the provider name after a slash was split off, which is what LiteLLM logs and maps with
    pub(crate) name: String,
    pub(crate) provider: ProviderId,
    pub(crate) config: ReadConfig<'deps>,
}

// B174 turns the declared name into a provider and an api_type, then B010's source selects that provider's configuration
pub(crate) fn target<'deps, Client>(
    operation: Operation,
    transport: &TransportRequest<'_>,
    deps: &VectorStoreReadDeps<'deps, Client>,
) -> Result<Target<'deps>, VectorStoreReadFailure> {
    let requested = transport.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
    let resolved =
        resolve_provider(operation.label(), transport.custom_llm_provider).map_err(|source| {
            let error = VectorStoreReadError::Resolution(source);
            failure(error, requested, deps.redactor)
        })?;
    let name = resolved.provider.to_string();
    let Some(config) = deps.configs.read_config(&resolved) else {
        let error = VectorStoreReadError::Unconfigured {
            operation,
            provider: name.clone(),
        };
        return Err(failure(error, &name, deps.redactor));
    };
    Ok(Target {
        operation,
        name,
        provider: resolved.provider,
        config,
    })
}

// `timeout or request_timeout`: an explicit zero falls back, and REQUEST_TIMEOUT is never absent
pub(crate) fn call_timeout(
    requested: Option<f64>,
    configured: f64,
) -> Result<Duration, VectorStoreReadError> {
    let seconds = requested
        .filter(|seconds| *seconds != 0.0_f64)
        .unwrap_or(configured);
    Duration::try_from_secs_f64(seconds)
        .map_err(|source| VectorStoreReadError::Timeout { seconds, source })
}

// retrieve's pre-call logging: the id alone goes into optional_params, and the read needs nothing more
pub(crate) fn retrieve_options<'request, Client>(
    deps: &VectorStoreReadDeps<'_, Client>,
    request: &'request VectorStoreReadRequest<'request>,
    provider: &str,
) -> Result<Content<'request>, VectorStoreReadFailure> {
    let optional_params = IndexMap::from([(
        String::from("vector_store_id"),
        Json::Text(request.vector_store_id.to_owned()),
    )]);
    log_call(
        deps,
        request.transport.kwargs,
        RETRIEVE_FLAG,
        optional_params,
        provider,
    )
    .map_err(|error| failure(error, provider, deps.redactor))?;
    Ok(Content {
        vector_store_id: Some(request.vector_store_id),
        query: ListQuery::default(),
    })
}

// list's pre-call logging: all four arguments, each present even when it is null
pub(crate) fn list_options<'request, Client>(
    deps: &VectorStoreReadDeps<'_, Client>,
    request: &'request VectorStoreListRequest<'request>,
    provider: &str,
) -> Result<Content<'request>, VectorStoreReadFailure> {
    // the signature's defaults: an absent limit and order become 20 and desc, while an explicit null stays absent
    let query = ListQuery {
        after: request.after,
        before: request.before,
        limit: request.limit.unwrap_or(Some(20)),
        order: request.order.unwrap_or(Some("desc")),
    };
    let optional_params = IndexMap::from([
        (
            String::from("after"),
            query
                .after
                .map_or(Json::Null, |after| Json::Text(after.to_owned())),
        ),
        (
            String::from("before"),
            query
                .before
                .map_or(Json::Null, |before| Json::Text(before.to_owned())),
        ),
        (
            String::from("limit"),
            query
                .limit
                .map_or(Json::Null, |limit| Json::Number(Number::from(limit))),
        ),
        (
            String::from("order"),
            query
                .order
                .map_or(Json::Null, |order| Json::Text(order.to_owned())),
        ),
    ]);
    log_call(
        deps,
        request.transport.kwargs,
        LIST_FLAG,
        optional_params,
        provider,
    )
    .map_err(|error| failure(error, provider, deps.redactor))?;
    Ok(Content {
        vector_store_id: None,
        query,
    })
}

// the async flag is popped before LiteLLM logs, so it is read and then kept out of the forwarded kwargs
fn log_call<Client>(
    deps: &VectorStoreReadDeps<'_, Client>,
    kwargs: &IndexMap<String, Json>,
    flag: &str,
    optional_params: IndexMap<String, Json>,
    provider: &str,
) -> Result<(), VectorStoreReadError> {
    let mut forwarded = kwargs.clone();
    forwarded.shift_remove(flag);
    let litellm_params = IndexMap::from([(
        String::from("litellm_call_id"),
        forwarded
            .get("litellm_call_id")
            .cloned()
            .unwrap_or(Json::Null),
    )]);
    let additional_params = IndexMap::from([(
        String::from("custom_llm_provider"),
        Json::Text(provider.to_owned()),
    )]);
    let mut state = deps.logging.lock().unwrap_or_else(PoisonError::into_inner);
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
    .map_err(VectorStoreReadError::Logging)
}
