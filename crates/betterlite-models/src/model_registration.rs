use std::error::Error;
use std::future::Future;

use indexmap::IndexMap;
use serde::Deserialize;

use crate::catalog_generalizations::CatalogGeneralizations;
use crate::decisions::Json;
use crate::model_catalog_keys::ModelCostKeys;
use crate::provider_model_sets::ProviderModelSets;
use error::{EntryError, RegistrationError, RegistrationWarning};
use merge::update_dictionary;
use seed::{Incoming, Seed, seed_entry};

pub mod builtin;
pub mod error;
pub mod merge;
mod provider_sets;
mod seed;
mod tests;

const COST_FIELDS: [&str; 2] = ["input_cost_per_token", "output_cost_per_token"];
const FETCH_ATTEMPTS: u32 = 1;

pub type JsonObject = IndexMap<String, Json>;
pub type ModelCostEntries = IndexMap<String, JsonObject>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelCostSource {
    Entries(ModelCostEntries),
    Url(String),
}

#[derive(Clone, Copy, Debug)]
pub struct RegisterOptions<'name> {
    pub persist_across_reloads: bool,
    pub warning_display_name: Option<&'name str>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveRegistration {
    pub entries: ModelCostEntries,
    pub persist_across_reloads: bool,
    pub warning_display_name: Option<String>,
}

#[derive(Debug)]
pub struct Registered {
    pub model_cost: ModelCostSource,
    pub warnings: Vec<RegistrationWarning>,
}

pub trait CostMapFetcher: Sync {
    type Error: Error + 'static;

    fn get_model_cost_map(
        &self,
        url: &str,
        max_attempts: u32,
    ) -> impl Future<Output = Result<ModelCostEntries, Self::Error>> + Send;
}

pub trait ModelInfoLookup: Sync {
    fn get_model_info(
        &self,
        model: &str,
        model_cost: &JsonObject,
        keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Option<JsonObject>> + Send;

    fn clear_caches(&self);
}

pub trait LiveDeploymentReplay: Send + Sync {
    fn registrations(&self) -> Vec<LiveRegistration>;
}

pub struct RegistrationEnvironment<'env, Lookup, Fetcher> {
    pub lookup: &'env Lookup,
    pub fetcher: &'env Fetcher,
    pub generalizations: &'env CatalogGeneralizations,
    pub bedrock_base_model: fn(&str) -> String,
}

#[derive(Default)]
pub struct ModelRegistry {
    pub model_cost: JsonObject,
    pub runtime_registered: ModelCostEntries,
    pub provider_sets: ProviderModelSets,
    pub keys: ModelCostKeys,
    pub live_replay: Option<Box<dyn LiveDeploymentReplay>>,
}

fn provider_of(key: &str, value: &JsonObject) -> Result<String, EntryError> {
    match value.get("litellm_provider") {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(provider)) => Ok(provider.clone()),
        Some(_) => Err(EntryError::ProviderNotText {
            key: key.to_owned(),
        }),
    }
}

impl ModelRegistry {
    #[must_use]
    pub fn cost_map_omits_token_price(&self, keys: &[&str]) -> bool {
        let entries: Vec<&JsonObject> = keys
            .iter()
            .filter_map(|key| match self.model_cost.get(*key) {
                Some(Json::Object(entry)) => Some(entry),
                _ => None,
            })
            .collect();
        !entries.is_empty()
            && !entries
                .iter()
                .any(|entry| COST_FIELDS.iter().any(|field| entry.contains_key(*field)))
    }

    pub async fn register_model<Lookup, Fetcher>(
        &mut self,
        model_cost: ModelCostSource,
        options: &RegisterOptions<'_>,
        environment: &RegistrationEnvironment<'_, Lookup, Fetcher>,
    ) -> Result<Registered, RegistrationError<Fetcher::Error>>
    where
        Lookup: ModelInfoLookup,
        Fetcher: CostMapFetcher,
    {
        let fetched;
        let entries = match &model_cost {
            ModelCostSource::Entries(entries) => entries,
            ModelCostSource::Url(url) => {
                let loaded = environment
                    .fetcher
                    .get_model_cost_map(url, FETCH_ATTEMPTS)
                    .await;
                fetched = loaded.map_err(|source| RegistrationError::Fetch {
                    url: url.clone(),
                    source,
                })?;
                &fetched
            }
        };
        if options.persist_across_reloads {
            self.runtime_registered.extend(entries.clone());
        }
        let mut warnings = Vec::new();
        for (key, value) in entries {
            let provider = provider_of(key, value)?;
            let incoming = Incoming {
                key,
                provider: &provider,
                value,
                display_name: options.warning_display_name.unwrap_or(key),
            };
            let seed = seed_entry(&self.model_cost, &mut self.keys, &incoming, environment).await?;
            warnings.extend(self.merge_entry(seed, &incoming));
            self.keys.lowercase_index = None;
            self.keys.generation = self.keys.generation.saturating_add(1);
            environment.lookup.clear_caches();
            provider_sets::add_registered_model(&mut self.provider_sets, &provider, key);
        }
        Ok(Registered {
            model_cost,
            warnings,
        })
    }

    fn merge_entry(&mut self, seed: Seed, incoming: &Incoming<'_>) -> Option<RegistrationWarning> {
        let Seed {
            mut existing,
            model_cost_key,
            warning,
        } = seed;
        // a None provider written back makes later cost lookups drop the custom pricing
        if existing
            .get("litellm_provider")
            .is_none_or(|provider| matches!(provider, Json::Null))
        {
            existing.shift_remove("litellm_provider");
        }
        // issue 30198: get_model_info fills a missing price with 0, and a stored zero reads as free
        let raw_entry = self
            .model_cost
            .get(&model_cost_key)
            .or_else(|| self.model_cost.get(incoming.key));
        for field in COST_FIELDS {
            let in_raw = matches!(raw_entry, Some(Json::Object(raw)) if raw.contains_key(field));
            if !in_raw && !incoming.value.contains_key(field) {
                existing.shift_remove(field);
            }
        }
        update_dictionary(&mut existing, incoming.value);
        let live = self
            .model_cost
            .entry(model_cost_key)
            .or_insert_with(|| Json::Object(IndexMap::new()));
        if let Json::Object(entry) = live {
            entry.extend(existing);
        } else {
            *live = Json::Object(existing);
        }
        warning
    }

    pub async fn reapply_runtime_registrations<Lookup, Fetcher>(
        &mut self,
        environment: &RegistrationEnvironment<'_, Lookup, Fetcher>,
    ) -> Result<Vec<RegistrationWarning>, RegistrationError<Fetcher::Error>>
    where
        Lookup: ModelInfoLookup,
        Fetcher: CostMapFetcher,
    {
        let live = self
            .live_replay
            .as_ref()
            .map_or_else(Vec::new, |replay| replay.registrations());
        let mut warnings = Vec::new();
        for registration in live {
            let options = RegisterOptions {
                persist_across_reloads: registration.persist_across_reloads,
                warning_display_name: registration.warning_display_name.as_deref(),
            };
            let source = ModelCostSource::Entries(registration.entries);
            let registered = self.register_model(source, &options, environment).await?;
            warnings.extend(registered.warnings);
        }
        if !self.runtime_registered.is_empty() {
            let options = RegisterOptions {
                persist_across_reloads: true,
                warning_display_name: None,
            };
            let source = ModelCostSource::Entries(self.runtime_registered.clone());
            let registered = self.register_model(source, &options, environment).await?;
            warnings.extend(registered.warnings);
        }
        Ok(warnings)
    }
}
