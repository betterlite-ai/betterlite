use std::sync::Arc;

use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::CallUpdate;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::logging::{generic_params_dump, log_update};
use super::{
    OptionalParamFilters, SdkDeps, SearchCall, call_duration, merge_optionals, query_json, supplied,
};
use crate::api::vector_search_execution::config::{Params, SearchConfig};
use crate::api::vector_search_execution::service::{
    SearchCall as SearchTransportCall, SearchDeps, prepare as prepare_search, send as send_search,
};
use crate::api::vector_store_http_write::service::{ClientSource, LogContext, WriteTransport};
use crate::api::vector_store_search::error::VectorStoreSearchError;
use crate::api::vector_store_search::executor::{SelectedExecutor, direct_executor};
use crate::api::vector_store_search::mock;
use crate::api::vector_store_search::provider::{ResolvedProvider, resolve_provider};

// search/asearch: the executor, the registry, the mock, the provider and its config, the filtered fields and B2484's search
pub async fn search<S>(
    deps: &SdkDeps<'_, S>,
    call: &SearchCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<Json, VectorStoreSearchError>
where
    S: ClientSource<Client: WriteTransport>,
{
    // the private argument is popped, so the executor selection, the log locals and the request never see it
    let mut kwargs = call.kwargs.clone();
    kwargs.remove("_direct_vector_store_embedding_executor");
    let selected = direct_executor(call.executor, deps.executors, &kwargs)?;
    kwargs.remove("asearch");
    if let Some(registry) = deps.registry {
        kwargs.extend(registry.credentials_for_vector_store(call.vector_store_id));
    }
    if let Some(answer) = search_mock(&kwargs) {
        return Ok(answer);
    }
    let resolved = resolve_provider("search", call.custom_llm_provider)?;
    let filtered = search_fields(deps.filters, call, &kwargs, resolved.tag);
    log_search(log.state, call, &kwargs, &filtered, &resolved)?;
    let parts = SearchParts {
        kwargs: &kwargs,
        filtered: &filtered,
        resolved: &resolved,
    };
    search_send(deps, call, log, &parts, &selected).await
}

// local_vars carries the named arguments too, which the provider's map_params reads as non_default_params
fn search_fields(
    filters: &dyn OptionalParamFilters,
    call: &SearchCall<'_>,
    kwargs: &Params,
    tag: VectorStoresConfigTag,
) -> Params {
    let mut merged = merge_optionals(call.optional, kwargs);
    merged.insert(
        "vector_store_id".to_owned(),
        Json::Text(call.vector_store_id.to_owned()),
    );
    merged.insert("query".to_owned(), query_json(call.query));
    filters.search_params(&merged, tag)
}

// what B2484's search reads: the request kwargs after the removals, the filtered fields and the provider
struct SearchParts<'parts> {
    kwargs: &'parts Params,
    filtered: &'parts Params,
    resolved: &'parts ResolvedProvider,
}

async fn search_send<S>(
    deps: &SdkDeps<'_, S>,
    call: &SearchCall<'_>,
    log: &mut LogContext<'_, '_>,
    parts: &SearchParts<'_>,
    selected: &SelectedExecutor<'_>,
) -> Result<Json, VectorStoreSearchError>
where
    S: ClientSource<Client: WriteTransport>,
{
    let (config, provider) = search_config(deps, parts.resolved)?;
    let timeout = call_duration(call.timeout, deps.settings)?;
    let transport = SearchTransportCall {
        vector_store_id: call.vector_store_id,
        query: call.query,
        optional_params: parts.filtered,
        litellm_params: parts.kwargs,
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
        &transport,
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

// B010's search config for the resolved tag, or the local unsupported ValueError the SDK raises before HTTP
fn search_config<S>(
    deps: &SdkDeps<'_, S>,
    resolved: &ResolvedProvider,
) -> Result<(Arc<dyn SearchConfig>, String), VectorStoreSearchError>
where
    S: ClientSource,
{
    let provider = resolved.provider.to_string();
    let config = deps.configs.search_config(resolved.tag).ok_or_else(|| {
        VectorStoreSearchError::Unsupported {
            operation: "search",
            provider: provider.clone(),
        }
    })?;
    Ok((config, provider))
}

// mock_vector_store_search_response: a nonempty list replaces data exactly, a nonempty string takes LiteLLM's sample
fn search_mock(kwargs: &Params) -> Option<Json> {
    let mock = kwargs
        .get("mock_response")
        .filter(|value| !matches!(value, Json::Null));
    if let Some(Json::List(items)) = mock
        && !items.is_empty()
    {
        return Some(mock::mock_search_response(mock));
    }
    if let Some(Json::Text(sample)) = mock
        && !sample.is_empty()
    {
        return Some(mock::mock_search_response(None));
    }
    None
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
        &litellm_params,
        CallUpdate {
            model: resolved.api_type.clone(),
            user: None,
            optional_params,
            additional_params,
        },
    )
}
