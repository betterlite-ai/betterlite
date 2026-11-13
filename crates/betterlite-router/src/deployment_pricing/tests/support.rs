use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex};

use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::decisions::Json;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_registration::{
    CostMapFetcher, JsonObject, ModelCostEntries, ModelInfoLookup, ModelRegistry,
    RegistrationEnvironment,
};
use betterlite_models::provider_model_sets::ProviderModelSets;
use serde_json::Number;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::deployment_pricing::PricingInputs;
use crate::deployment_pricing::deployment::{DeploymentReadError, DeploymentTarget};
use crate::deployment_pricing::inherit::BackendModel;
use crate::deployment_pricing::payload::deployment_cost_payload;

pub(super) const COST_MAP: &str = r#"{
    "claude-x": {
        "litellm_provider": "anthropic",
        "mode": "chat",
        "input_cost_per_token": 0.000003,
        "output_cost_per_token": 0.000015,
        "cache_read_input_token_cost": 0.0000003,
        "cache_creation_input_token_cost": 0.00000375,
        "cache_creation_input_token_cost_above_1hr": 0.000006,
        "input_cost_per_token_flex": 0.0000015,
        "input_cost_per_token_priority": 0.000006,
        "output_cost_per_token_priority": null,
        "supports_vision": true
    },
    "tiered-only": {
        "litellm_provider": "openai",
        "mode": "chat",
        "tiered_pricing": [{"range": [0, 1000], "input_cost_per_token": 0.000001}],
        "cache_read_input_token_cost": 0.0000001
    },
    "gpt-x": {
        "litellm_provider": "openai",
        "mode": "chat",
        "input_cost_per_token": 0.000002,
        "output_cost_per_token": 0.000008
    },
    "tiers-all": {
        "litellm_provider": "openai",
        "a_balanced": 1,
        "b_flex": 2,
        "c_priority": 3,
        "d_ultrafast": 4,
        "e_fast": 5,
        "f_priority_extra": 6,
        "g_flexible": 7
    },
    "zero-rate": {"litellm_provider": "openai", "output_cost_per_token": 0.0},
    "negative-zero-rate": {"litellm_provider": "openai", "output_cost_per_token": -0.0},
    "tiny-rate": {"litellm_provider": "openai", "output_cost_per_token": 1e-7}
}"#;

pub(super) fn is_auto_router(model: &str) -> bool {
    model.starts_with("auto_router/")
}

pub(super) const INPUTS: PricingInputs = PricingInputs {
    ptu_cost_attribution: false,
    is_strategy_router: is_auto_router,
};

pub(super) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn registry(cost_map: &str) -> ModelRegistry {
    ModelRegistry {
        model_cost: object(cost_map),
        runtime_registered: ModelCostEntries::new(),
        provider_sets: ProviderModelSets::default(),
        keys: ModelCostKeys::default(),
        live_replay: None,
    }
}

pub(super) const PTU_TERMS: &str = r#"{"team_id": "t", "ptu_count": 10, "cost_per_ptu_per_hour": 2, "ptu_effective_from": "2026-01-01T00:00:00Z"}"#;

pub(super) fn backend<'name>(
    model: &'name str,
    provider: Option<&'name str>,
) -> BackendModel<'name> {
    BackendModel {
        model,
        custom_llm_provider: provider,
    }
}

pub(super) fn merged(base: &str, extra: &str) -> JsonObject {
    let mut fields = object(base);
    fields.extend(object(extra));
    fields
}

pub(super) const CUSTOM_BACKEND: DeploymentTarget<'static> = DeploymentTarget {
    model_id: Some("dep-1"),
    model: "custom-m",
    custom_llm_provider: Some("acme"),
};

pub(super) fn deployment(id: &str, mode: &str) -> Json {
    json(&format!(
        r#"{{"model_name": "g", "litellm_params": {{"model": "custom-m", "custom_llm_provider": "acme"}}, "model_info": {{"id": "{id}", "mode": "{mode}"}}}}"#
    ))
}

pub(super) async fn cost_payload(
    world: &World,
    litellm_params: &JsonObject,
    model_info: &JsonObject,
    inputs: &PricingInputs,
) -> Result<JsonObject, DeploymentReadError> {
    let mut registry = registry(COST_MAP);
    deployment_cost_payload(
        litellm_params,
        model_info,
        &mut registry,
        &world.lookup,
        inputs,
    )
    .await
}

// Finds an entry by its exact name or without a provider prefix, and fills a missing flat rate with 0 like get_model_info.
pub(super) struct CostMapLookup {
    pub(super) queries: Mutex<Vec<String>>,
    pub(super) omit_key: bool,
}

impl CostMapLookup {
    pub(super) fn new() -> Self {
        Self {
            queries: Mutex::new(Vec::new()),
            omit_key: false,
        }
    }

    pub(super) fn queried(&self) -> Vec<String> {
        self.queries.lock().unwrap().clone()
    }
}

impl ModelInfoLookup for CostMapLookup {
    fn get_model_info(
        &self,
        model: &str,
        model_cost: &JsonObject,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Option<JsonObject>> + Send {
        self.queries.lock().unwrap().push(model.to_owned());
        let bare = model
            .split_once('/')
            .map_or(model, |(_provider, rest)| rest);
        let found = [model, bare]
            .into_iter()
            .find_map(|key| match model_cost.get(key) {
                Some(Json::Object(entry)) => Some((key, entry)),
                _ => None,
            });
        ready(found.map(|(key, entry)| {
            let mut info = entry.clone();
            if !self.omit_key {
                info.insert("key".to_owned(), Json::Text(key.to_owned()));
            }
            for field in ["input_cost_per_token", "output_cost_per_token"] {
                let rate = match info.get(field) {
                    None | Some(Json::Null) => Json::Number(Number::from(0_u8)),
                    Some(stored) => stored.clone(),
                };
                info.insert(field.to_owned(), rate);
            }
            info
        }))
    }

    fn clear_caches(&self) {}
}

pub(super) struct NoFetch;

#[derive(Debug, thiserror::Error)]
#[error("these tests never fetch a cost map")]
pub(super) struct NoFetchError;

impl CostMapFetcher for NoFetch {
    type Error = NoFetchError;

    fn get_model_cost_map(
        &self,
        _url: &str,
        _max_attempts: u32,
    ) -> impl Future<Output = Result<ModelCostEntries, NoFetchError>> + Send {
        ready(Err(NoFetchError))
    }
}

pub(super) struct World {
    pub(super) lookup: CostMapLookup,
    fetcher: NoFetch,
    generalizations: CatalogGeneralizations,
}

impl World {
    pub(super) fn new() -> Self {
        Self {
            lookup: CostMapLookup::new(),
            fetcher: NoFetch,
            generalizations: CatalogGeneralizations::default(),
        }
    }

    pub(super) fn environment(&self) -> RegistrationEnvironment<'_, CostMapLookup, NoFetch> {
        RegistrationEnvironment {
            lookup: &self.lookup,
            fetcher: &self.fetcher,
            generalizations: &self.generalizations,
            bedrock_base_model: str::to_owned,
        }
    }
}

#[derive(Clone)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|poisoned| io::Error::other(poisoned.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    pub(super) fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

pub(super) fn capture_logs() -> (Captured, DefaultGuard) {
    let captured = Captured(Arc::new(Mutex::new(Vec::new())));
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}
