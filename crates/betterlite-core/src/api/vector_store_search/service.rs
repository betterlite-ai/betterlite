use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::api::vector_store_search::Settings;
use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::store::CreateParams;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::error::VectorStoreSearchError;
use super::executor::{
    DirectExecutor, EmbeddingExecutorSource, direct_vector_store_embedding_executor,
};
use super::mock;
use super::provider::{ResolvedProvider, resolve_provider};
use crate::api::vector_search_execution::config::{Params, SearchConfig, SearchQuery};
use crate::api::vector_search_execution::service::signing::SigningSeam;
use crate::api::vector_search_execution::service::{
    SearchCall as SearchTransportCall, SearchDeps, prepare as prepare_search, send as send_search,
};
use crate::api::vector_store_http_create_read::config::{CapabilityError, CreateReadConfig};
use crate::api::vector_store_http_create_read::service::{
    CreateCall as CreateTransportCall, CreateReadCall, CreateReadDeps, CreateReadTarget,
    prepare as prepare_create, send as send_create,
};
use crate::api::vector_store_http_write::config::{WriteConfig, WriteParams};
use crate::api::vector_store_http_write::service::{
    ClientKind, ClientSource, LogContext, SuppliedClient,
};
use crate::api::vector_store_registry::store::VectorStoreRegistry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Sync,
    Asynchronous,
}

impl Entry {
    const fn client_kind(self) -> ClientKind {
        match self {
            Self::Sync => ClientKind::Sync,
            Self::Asynchronous => ClientKind::Asynchronous,
        }
    }
}

// kwargs.pop("asearch")/kwargs.pop("acreate") is read as `is True`: a 1, a truthy string or a false selects the sync handler
#[must_use]
pub fn entry_from_flag(kwargs: &Params, flag: &'static str) -> Entry {
    match kwargs.get(flag) {
        Some(Json::Bool(true)) => Entry::Asynchronous,
        Some(
            Json::Bool(false)
            | Json::Null
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => Entry::Sync,
    }
}

// B1058: the declared keys whose values are not null, then the provider config's map_params for a search
pub trait OptionalParamFilters {
    fn create_params(&self, params: &Params) -> Params;
    fn search_params(&self, params: &Params, tag: VectorStoresConfigTag) -> Params;
}

// The provider configs B010 selected, which the caller builds for the tags it serves
pub trait VectorStoreConfigs {
    fn create_config(&self, tag: VectorStoresConfigTag) -> Option<Box<dyn CreateReadConfig + '_>>;
    fn search_config(&self, tag: VectorStoresConfigTag) -> Option<Arc<dyn SearchConfig>>;
}

// B2485 takes any sized config, so a config the caller built behind a box is lent as one
pub struct OwnedCreateReadConfig<'config> {
    inner: Box<dyn CreateReadConfig + 'config>,
}

impl CreateReadConfig for OwnedCreateReadConfig<'_> {
    fn validate_create(&self) -> Result<(), CapabilityError> {
        self.inner.validate_create()
    }

    fn provider(&self) -> &dyn WriteConfig {
        self.inner.provider()
    }
}

pub struct SdkDeps<'deps, S: ClientSource> {
    pub configs: &'deps dyn VectorStoreConfigs,
    pub filters: &'deps dyn OptionalParamFilters,
    pub executors: &'deps dyn EmbeddingExecutorSource,
    pub signing: &'deps dyn SigningSeam,
    pub registry: Option<&'deps VectorStoreRegistry>,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
    pub settings: Settings,
}

// vector_store_create's named optional arguments plus the call's **kwargs
pub struct CreateCall<'call> {
    pub optional: &'call Params,
    pub kwargs: &'call Params,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<&'call str>,
    pub entry: Entry,
}

// vector_store_search's named optional arguments, its **kwargs and the private executor argument
pub struct SearchCall<'call> {
    pub vector_store_id: &'call str,
    pub query: &'call SearchQuery,
    pub optional: &'call Params,
    pub kwargs: &'call Params,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub extra_body: Option<&'call Params>,
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<&'call str>,
    pub executor: Option<DirectExecutor<'call>>,
    pub entry: Entry,
}

// create/acreate: the mock, then the provider and its config, the filtered fields, the pre-call and B2485's create
pub async fn create<S>(
    deps: &SdkDeps<'_, S>,
    call: &CreateCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<Json, VectorStoreSearchError>
where
    S: ClientSource,
{
    let mut kwargs = call.kwargs.clone();
    kwargs.remove("acreate");
    let api_key = text(&kwargs, "api_key")?;
    let api_base = text(&kwargs, "api_base")?;
    let ssl_verify = flag(&kwargs, "ssl_verify")?;
    let mock_response = kwargs
        .get("mock_response")
        .filter(|value| !matches!(value, Json::Null));
    if let Some(given @ Json::Object(fields)) = mock_response
        && !fields.is_empty()
    {
        return Ok(mock::mock_create_response(Some(given)));
    }
    let resolved = resolve_provider("create", call.custom_llm_provider)?;
    let provider = resolved.provider.to_string();
    let config = OwnedCreateReadConfig {
        inner: deps.configs.create_config(resolved.tag).ok_or_else(|| {
            VectorStoreSearchError::Unsupported {
                operation: "create",
                provider: provider.clone(),
            }
        })?,
    };
    let merged = merge_optionals(call.optional, &kwargs);
    let filtered = deps.filters.create_params(&merged);
    let params = create_params(&filtered)?;
    log_create(log.state, call, &kwargs, &filtered, &provider)?;
    let timeout = call_duration(call.timeout, deps.settings)?;
    let write = WriteParams {
        api_base,
        api_key: api_key.map(SecretString::from),
        ssl_verify,
    };
    let target = CreateReadTarget {
        params: &write,
        extra_headers: call.extra_headers,
        timeout: Some(timeout),
    };
    let transport_call = CreateReadCall::Create(CreateTransportCall {
        target,
        optional_params: &params,
        extra_body: call.extra_body,
    });
    let create_deps = CreateReadDeps {
        config: &config,
        source: deps.source,
        supplied: supplied(deps),
    };
    let prepared = prepare_create(&create_deps, call.entry.client_kind(), &transport_call, log)
        .map_err(|source| VectorStoreSearchError::Create {
            provider: provider.clone(),
            source,
        })?;
    send_create(prepared)
        .await
        .map_err(|source| VectorStoreSearchError::Create { provider, source })
}

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
    let selected = direct_vector_store_embedding_executor(call.executor, deps.executors, &kwargs)?;
    kwargs.remove("asearch");
    if let Some(registry) = deps.registry {
        kwargs.extend(registry.credentials_for_vector_store(call.vector_store_id));
    }
    let mock_response = kwargs
        .get("mock_response")
        .filter(|value| !matches!(value, Json::Null));
    let mocked = match mock_response {
        Some(Json::Text(text)) => !text.is_empty(),
        Some(Json::List(items)) => !items.is_empty(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) | None => false,
    };
    if mocked {
        let results = match mock_response {
            Some(list @ Json::List(_)) => Some(list),
            Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_),
            )
            | None => None,
        };
        return Ok(mock::mock_search_response(results));
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

fn supplied<'deps, S>(deps: &SdkDeps<'deps, S>) -> Option<SuppliedClient<'deps, S::Client>>
where
    S: ClientSource,
{
    match deps.supplied.as_ref() {
        None => None,
        Some(held) => Some(SuppliedClient {
            kind: held.kind,
            transport: held.transport,
        }),
    }
}

// VectorStoreCreateRequest's five fields; B1058 has already dropped everything else
fn create_params(filtered: &Params) -> Result<CreateParams, VectorStoreSearchError> {
    let invalid = VectorStoreSearchError::Field {
        field: "optional_params",
        expected: "the five declared create fields",
    };
    let value = serde_json::to_value(filtered)?;
    serde_json::from_value(value).map_err(|_| invalid)
}

// GenericLiteLLMParams(**kwargs): api_key and api_base are strings, and a value of another type names the field
fn text(kwargs: &Params, field: &'static str) -> Result<Option<String>, VectorStoreSearchError> {
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(value)) => Ok(Some(value.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(VectorStoreSearchError::Field {
                field,
                expected: "a string",
            })
        }
    }
}

fn flag(kwargs: &Params, field: &'static str) -> Result<Option<bool>, VectorStoreSearchError> {
    match kwargs.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(Json::Text(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(VectorStoreSearchError::Field {
                field,
                expected: "a boolean",
            })
        }
    }
}

// timeout or request_timeout: a null or zero argument uses the setting, and any other value is the wait in seconds
fn call_duration(
    given: Option<f64>,
    settings: Settings,
) -> Result<Duration, VectorStoreSearchError> {
    let seconds = given
        .filter(|value| *value != 0.0)
        .unwrap_or(settings.request_timeout_seconds);
    let duration = Duration::try_from_secs_f64(seconds);
    duration.map_err(VectorStoreSearchError::Timeout)
}

// local_vars.update(kwargs): every keyword overrides the named argument of the same name before B1058 filters
fn merge_optionals(optional: &Params, kwargs: &Params) -> Params {
    let mut merged = optional.clone();
    for (name, value) in kwargs {
        merged.insert(name.clone(), value.clone());
    }
    merged
}

fn as_index_map(params: &Params) -> IndexMap<String, Json> {
    let mut index = IndexMap::new();
    for (name, value) in params {
        index.insert(name.clone(), value.clone());
    }
    index
}

// GenericLiteLLMParams.model_dump(exclude_none=True): the five fields defaulting to False, then every non-null kwarg
fn generic_params_dump(kwargs: &Params) -> Params {
    let mut dump = Params::from([
        (
            "allow_client_keepalive_override".to_owned(),
            Json::Bool(false),
        ),
        (
            "merge_reasoning_content_in_choices".to_owned(),
            Json::Bool(false),
        ),
        ("use_in_pass_through".to_owned(), Json::Bool(false)),
        ("use_litellm_proxy".to_owned(), Json::Bool(false)),
        ("use_xai_oauth".to_owned(), Json::Bool(false)),
    ]);
    for (name, value) in kwargs {
        if !matches!(value, Json::Null) {
            dump.insert(name.clone(), value.clone());
        }
    }
    dump
}

// update_from_kwargs(model=None, optional_params={"name": name, **filtered}, litellm_params={"litellm_call_id": ...})
fn log_create(
    state: &mut LoggingState,
    call: &CreateCall<'_>,
    kwargs: &Params,
    filtered: &Params,
    provider: &str,
) -> Result<(), VectorStoreSearchError> {
    let mut optional_params = IndexMap::new();
    optional_params.insert(
        "name".to_owned(),
        call.optional.get("name").cloned().unwrap_or(Json::Null),
    );
    for (name, value) in filtered {
        optional_params.insert(name.clone(), value.clone());
    }
    let litellm_params = IndexMap::from([(
        "litellm_call_id".to_owned(),
        kwargs.get("litellm_call_id").cloned().unwrap_or(Json::Null),
    )]);
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(provider.to_owned()),
    );
    update_from_kwargs(
        state,
        &as_index_map(kwargs),
        Some(&litellm_params),
        CallUpdate {
            model: None,
            user: None,
            optional_params,
            additional_params,
        },
    )?;
    Ok(())
}

// update_from_kwargs(model=api_type, optional_params={id, query, **filtered}, litellm_params={id, call id, **model_dump})
fn log_search(
    state: &mut LoggingState,
    call: &SearchCall<'_>,
    kwargs: &Params,
    filtered: &Params,
    resolved: &ResolvedProvider,
) -> Result<(), VectorStoreSearchError> {
    let mut optional_params = IndexMap::new();
    optional_params.insert(
        "vector_store_id".to_owned(),
        Json::Text(call.vector_store_id.to_owned()),
    );
    optional_params.insert("query".to_owned(), query_json(call.query));
    for (name, value) in filtered {
        optional_params.insert(name.clone(), value.clone());
    }
    let mut litellm_params = IndexMap::new();
    litellm_params.insert(
        "litellm_call_id".to_owned(),
        kwargs.get("litellm_call_id").cloned().unwrap_or(Json::Null),
    );
    litellm_params.insert(
        "vector_store_id".to_owned(),
        Json::Text(call.vector_store_id.to_owned()),
    );
    for (name, value) in generic_params_dump(kwargs) {
        litellm_params.insert(name, value);
    }
    let mut additional_params = IndexMap::new();
    additional_params.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(resolved.provider.to_string()),
    );
    update_from_kwargs(
        state,
        &as_index_map(kwargs),
        Some(&litellm_params),
        CallUpdate {
            model: resolved.api_type.clone(),
            user: None,
            optional_params,
            additional_params,
        },
    )?;
    Ok(())
}

fn query_json(query: &SearchQuery) -> Json {
    match query {
        SearchQuery::Text(text) => Json::Text(text.clone()),
        SearchQuery::Sequence(parts) => {
            Json::List(parts.iter().map(|part| Json::Text(part.clone())).collect())
        }
    }
}
