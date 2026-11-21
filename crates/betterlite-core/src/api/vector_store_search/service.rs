use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::api::vector_store_search::Settings;
use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::error::VectorStoreSearchError;
use super::executor::{DirectExecutor, EmbeddingExecutorSource};
use crate::api::vector_search_execution::config::{Params, SearchConfig, SearchQuery};
use crate::api::vector_search_execution::service::signing::SigningSeam;
use crate::api::vector_store_http_create_read::{config::CreateReadConfig, error::CapabilityError};
use crate::api::vector_store_http_write::config::WriteConfig;
use crate::api::vector_store_http_write::service::{ClientKind, ClientSource, SuppliedClient};
use crate::api::vector_store_registry::store::VectorStoreRegistry;

pub mod create_call;
mod logging;
pub mod search_call;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Entry {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}

impl Entry {
    pub(super) const fn client_kind(self) -> ClientKind {
        match self {
            Self::Sync => ClientKind::Sync,
            Self::Asynchronous => ClientKind::Asynchronous,
        }
    }
}

// kwargs.pop("asearch")/kwargs.pop("acreate") is read as `is True`: a 1, a truthy string or a false selects the sync handler
#[must_use]
pub fn entry_from_flag(kwargs: &Params, flag: &'static str) -> Entry {
    if matches!(kwargs.get(flag), Some(Json::Bool(true))) {
        return Entry::Asynchronous;
    }
    Entry::Sync
}

// B1058: the declared keys whose values are not null, then the provider config's map_params for a search
pub trait OptionalParamFilters: Send + Sync {
    fn create_params(&self, params: &Params) -> Params;
    fn search_params(&self, params: &Params, tag: VectorStoresConfigTag) -> Params;
}

// The provider configs B010 selected, which the caller builds for the tags it serves
pub trait VectorStoreConfigs: Send + Sync {
    fn create_config(&self, tag: VectorStoresConfigTag) -> Option<Box<dyn CreateReadConfig + '_>>;
    fn search_config(&self, tag: VectorStoresConfigTag) -> Option<Arc<dyn SearchConfig>>;
}

// B2485 takes any sized config, so a config the caller built behind a box is lent as one
pub struct OwnedCreateReadConfig<'config> {
    pub(super) inner: Box<dyn CreateReadConfig + 'config>,
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

// B2485 borrows the client behind whatever shape it came in
pub(super) fn supplied<'deps, S>(
    deps: &SdkDeps<'deps, S>,
) -> Option<SuppliedClient<'deps, S::Client>>
where
    S: ClientSource,
{
    deps.supplied.as_ref().map(|held| SuppliedClient {
        kind: held.kind,
        transport: held.transport,
    })
}

// GenericLiteLLMParams(**kwargs): api_key and api_base are strings, and a value of another type names the field
pub(super) fn text(
    kwargs: &Params,
    field: &'static str,
) -> Result<Option<String>, VectorStoreSearchError> {
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

// local_vars.update(kwargs): every keyword overrides the named argument of the same name before B1058 filters
pub(super) fn merge_optionals(optional: &Params, kwargs: &Params) -> Params {
    let mut merged = optional.clone();
    merged.extend(
        kwargs
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    merged
}

// timeout or request_timeout: a null or zero argument uses the setting, and any other value is the wait in seconds
pub(super) fn call_duration(
    given: Option<f64>,
    settings: Settings,
) -> Result<Duration, VectorStoreSearchError> {
    let seconds = given
        .filter(|value| *value != 0.0_f64)
        .unwrap_or(settings.request_timeout_seconds);
    Duration::try_from_secs_f64(seconds).map_err(VectorStoreSearchError::Timeout)
}

pub(super) fn query_json(query: &SearchQuery) -> Json {
    match query {
        SearchQuery::Text(text) => Json::Text(text.clone()),
        SearchQuery::Sequence(parts) => {
            Json::List(parts.iter().map(|part| Json::Text(part.clone())).collect())
        }
    }
}
