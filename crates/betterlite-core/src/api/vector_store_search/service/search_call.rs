use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::logging::{generic_params_dump, log_update};
use super::{SdkDeps, SearchCall, call_duration, merge_optionals, query_json, supplied};
use crate::api::vector_search_execution::config::Params;
use crate::api::vector_search_execution::service::{
    SearchCall as SearchTransportCall, SearchDeps, prepare as prepare_search, send as send_search,
};
use crate::api::vector_store_http_write::service::{ClientSource, LogContext};
use crate::api::vector_store_search::error::VectorStoreSearchError;
use crate::api::vector_store_search::executor::direct_executor;
use crate::api::vector_store_search::mock;
use crate::api::vector_store_search::provider::{ResolvedProvider, resolve_provider};

// search/asearch: the executor, the registry, the mock, the provider and its config, the filtered fields and B2484's search
pub async fn search<S>(
    deps: &SdkDeps<'_, S>,
    call: &SearchCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<Json, VectorStoreSearchError>
where
    S: ClientSource,
{
    // the private argument is popped, so the executor selection, the log locals and the request never see it
    let mut kwargs = call.kwargs.clone();
    kwargs.remove("_direct_vector_store_embedding_executor");
    let selected = direct_executor(call.executor, deps.executors, &kwargs)?;
    kwargs.remove("asearch");
    if let Some(registry) = deps.registry {
        kwargs.extend(registry.credentials_for_vector_store(call.vector_store_id));
    }
    let mock_response = kwargs
        .get("mock_response")
        .filter(|value| !matches!(value, Json::Null));
    if let Some(Json::List(items)) = mock_response
        && !items.is_empty()
    {
        return Ok(mock::mock_search_response(Some(Json::List(items.clone()))));
    }
    if let Some(Json::Text(sample)) = mock_response
        && !sample.is_empty()
    {
        return Ok(mock::mock_search_response(None));
    }
    let resolved = resolve_provider("search", call.custom_llm_provider)?;
    let provider = resolved.provider.to_string();
    let config = deps.configs.search_config(resolved.tag).ok_or_else(|| {
        VectorStoreSearchError::Unsupported {
            operation: "search",
            provider: provider.clone(),
        }
    })?;
    // local_vars carries the named arguments too, which the provider's map_params reads as non_default_params
    let mut merged = merge_optionals(call.optional, &kwargs);
    merged.insert(
        "vector_store_id".to_owned(),
        Json::Text(call.vector_store_id.to_owned()),
    );
    merged.insert("query".to_owned(), query_json(call.query));
    let filtered = deps.filters.search_params(&merged, resolved.tag);
    log_search(log.state, call, &kwargs, &filtered, &resolved)?;
    let timeout = call_duration(call.timeout, deps.settings)?;
    let transport_call = SearchTransportCall {
        vector_store_id: call.vector_store_id,
        query: call.query,
        optional_params: &filtered,
        litellm_params: &kwargs,
        extra_headers: call.extra_headers,
        extra_body: call.extra_body,
        timeout: Some(timeout),
        embedding_executor: Some(selected.as_executor()),
    };
    let search_deps = SearchDeps {
        config,
        source: deps.source,
        supplied: supplied(deps),
    };
    let prepared = prepare_search(
        &search_deps,
        deps.signing,
        call.entry.client_kind(),
        &transport_call,
        log,
    )
    .await
    .map_err(|source| VectorStoreSearchError::Search {
        provider: provider.clone(),
        source,
    })?;
    let response = send_search(prepared)
        .await
        .map_err(|source| VectorStoreSearchError::Search { provider, source })?;
    let bytes = serde_json::to_vec(&response)?;
    Ok(serde_json::from_slice(&bytes)?)
}

// update_from_kwargs(model=api_type, optional_params={id, query, **filtered}, litellm_params={id, call id, **model_dump})
fn log_search(
    state: &mut LoggingState,
    call: &SearchCall<'_>,
    kwargs: &Params,
    filtered: &Params,
    resolved: &ResolvedProvider,
) -> Result<(), VectorStoreSearchError> {
    let mut optional_params = IndexMap::from([
        (
            "vector_store_id".to_owned(),
            Json::Text(call.vector_store_id.to_owned()),
        ),
        ("query".to_owned(), query_json(call.query)),
    ]);
    optional_params.extend(
        filtered
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    let mut litellm_params = IndexMap::from([(
        "litellm_call_id".to_owned(),
        kwargs.get("litellm_call_id").cloned().unwrap_or(Json::Null),
    )]);
    litellm_params.insert(
        "vector_store_id".to_owned(),
        Json::Text(call.vector_store_id.to_owned()),
    );
    litellm_params.extend(generic_params_dump(kwargs));
    let additional_params = IndexMap::from([(
        "custom_llm_provider".to_owned(),
        Json::Text(resolved.provider.to_string()),
    )]);
    log_update(
        state,
        kwargs,
        litellm_params,
        CallUpdate {
            model: resolved.api_type.clone(),
            user: None,
            optional_params,
            additional_params,
        },
    )
}
