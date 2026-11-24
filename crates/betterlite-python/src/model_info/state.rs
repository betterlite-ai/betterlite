use super::convert::RuntimeEntries;
use super::error::ModelInfoError;
use super::fetch::HttpFetch;
use super::map::{count_entries, finalize};
use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::catalog_source;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::fetch_policy::revision::git_blob_id;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_metadata_resolution::names::{
    ModelCostMap, NameNormalizers, VertexModelSets,
};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::source::{CatalogProvenance, SourceKind};
use chrono::{DateTime, SecondsFormat, Utc};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::value::RawValue;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
pub(crate) struct Bundled {
    pub(crate) catalog: Arc<ModelCatalog>,
    pub(crate) rules: Arc<CatalogGeneralizations>,
    pub(crate) finalized: IndexMap<String, Box<RawValue>>,
    pub(crate) revision: Option<String>,
    pub(crate) count: usize,
}
impl Bundled {
    fn parse(text: &str) -> Result<Self, ModelInfoError> {
        let ModelCostMap { entries, .. } = ModelCostMap::parse(text.as_bytes())?;
        let catalog = ModelCatalog::parse(text.as_bytes(), UnknownFields::Warn)?;
        let count = count_entries(&entries);
        let finalized = finalize(entries)?;
        let rules = Arc::new(CatalogGeneralizations::install(finalized.rules.clone()).0);
        Ok(Self {
            catalog: Arc::new(catalog),
            rules,
            finalized: finalized.entries,
            revision: Some(git_blob_id(text.as_bytes())),
            count,
        })
    }
}
#[derive(Clone)]
pub(crate) struct State {
    pub(crate) entries: IndexMap<String, Box<RawValue>>,
    pub(crate) vertex: VertexModelSets,
    pub(crate) catalog: Arc<ModelCatalog>,
    pub(crate) generalizations: Arc<CatalogGeneralizations>,
    pub(crate) provenance: CatalogProvenance,
    pub(crate) generation: u64,
}
pub(crate) struct Runtime {
    pub(crate) bundled: Bundled,
    environment: BTreeMap<String, String>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    registry: JSONProviderRegistry,
    pub(crate) fetch: HttpFetch,
    inner: Mutex<Arc<State>>,
}
impl Runtime {
    pub(crate) fn new(text: &str) -> Result<Self, ModelInfoError> {
        let environment = process_environment();
        let settings = catalog_settings(&environment)?;
        let fetch = HttpFetch::new(&environment)?;
        let bundled = Bundled::parse(text)?;
        let provenance = local_provenance(bundled.revision.clone(), settings.local, Utc::now());
        let state = initial_state(&bundled, provenance);
        Ok(Self {
            bundled,
            environment,
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            registry: JSONProviderRegistry::load(),
            fetch,
            inner: Mutex::new(Arc::new(state)),
        })
    }
    // ast-grep-ignore: no-padded-one-line-functions - the lock guard must outlive the clone
    pub(crate) fn state(&self) -> Arc<State> {
        let guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(&guard)
    }
    // ast-grep-ignore: no-padded-one-line-functions - the lock guard must outlive the change
    pub(crate) fn update<F>(&self, change: F)
    where
        F: FnOnce(&mut State),
    {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        change(Arc::make_mut(&mut guard));
    }
    pub(crate) fn resolution<'runtime>(
        &'runtime self,
        state: &'runtime State,
    ) -> ResolutionContext<'runtime> {
        ResolutionContext {
            catalog: &state.catalog,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
    pub(crate) fn apply_runtime_entries(&self, request: RuntimeEntries) {
        self.update(move |state| {
            for key in &request.removed {
                drop(state.entries.shift_remove(key));
            }
            for (key, value) in request.entries {
                drop(state.entries.insert(key, value));
            }
            state.vertex = VertexModelSets::build(&state.entries);
        });
    }
    pub(crate) fn adopt(&self, text: &str) -> Result<usize, ModelInfoError> {
        let entries: IndexMap<String, Box<RawValue>> = serde_json::from_str(text)?;
        let count = entries.len();
        let catalog = Arc::new(catalog_for(&entries));
        self.update(move |state| {
            state.vertex = VertexModelSets::build(&entries);
            state.catalog = catalog;
            state.entries = entries;
            state.generation = state.generation.saturating_add(1);
        });
        Ok(count)
    }
    pub(crate) fn source_json(&self) -> Result<String, ModelInfoError> {
        let state = self.state();
        let loaded_at = state
            .provenance
            .loaded_at
            .map(|value| value.to_rfc3339_opts(SecondsFormat::Micros, false));
        let info = SourceInfo {
            source: match state.provenance.kind {
                SourceKind::Local => "local",
                SourceKind::Remote => "remote",
            },
            url: state.provenance.url.as_deref(),
            is_env_forced: state.provenance.is_env_forced,
            fallback_reason: state.provenance.fallback_reason.as_deref(),
            loaded_at,
            source_revision: state.provenance.source_revision.as_deref(),
            etag: state.provenance.etag.as_deref(),
        };
        serde_json::to_string(&info).map_err(ModelInfoError::Result)
    }
}
fn initial_state(bundled: &Bundled, provenance: CatalogProvenance) -> State {
    State {
        entries: bundled.finalized.clone(),
        vertex: VertexModelSets::build(&bundled.finalized),
        catalog: Arc::clone(&bundled.catalog),
        generalizations: Arc::clone(&bundled.rules),
        provenance,
        generation: 0,
    }
}
pub(crate) const fn local_provenance(
    revision: Option<String>,
    is_env_forced: bool,
    loaded_at: DateTime<Utc>,
) -> CatalogProvenance {
    CatalogProvenance {
        kind: SourceKind::Local,
        url: None,
        is_env_forced,
        fallback_reason: None,
        loaded_at: Some(loaded_at),
        source_revision: revision,
        etag: None,
    }
}
// a cost map Python adopts keeps the lookup working even when the catalog rejects a field
fn catalog_for(entries: &IndexMap<String, Box<RawValue>>) -> ModelCatalog {
    serde_json::to_string(entries)
        .ok()
        .and_then(|text| ModelCatalog::parse(text.as_bytes(), UnknownFields::Warn).ok())
        .unwrap_or_default()
}
pub(crate) fn catalog_settings(
    environment: &BTreeMap<String, String>,
) -> Result<catalog_source::Settings, ModelInfoError> {
    catalog_source::Raw::default()
        .convert(environment)
        .map_err(|source| ModelInfoError::Settings { source })
}
pub(crate) fn normalizers() -> NameNormalizers {
    NameNormalizers {
        bedrock_base_model: get_bedrock_base_model,
        bedrock_routing_prefix: strip_bedrock_routing_prefix,
        mantle_region_split: split_region_prefix,
    }
}
#[derive(Serialize)]
struct SourceInfo<'source> {
    source: &'static str,
    url: Option<&'source str>,
    is_env_forced: bool,
    fallback_reason: Option<&'source str>,
    loaded_at: Option<String>,
    source_revision: Option<&'source str>,
    etag: Option<&'source str>,
}
mod tests;
