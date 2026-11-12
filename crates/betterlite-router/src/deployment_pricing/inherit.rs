use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::{JsonObject, ModelInfoLookup, ModelRegistry};
use betterlite_models::ptu_pricing::{PtuModelInfo, ptu_terms};
use betterlite_models::tiered_pricing::Scalar;
use serde::de::IgnoredAny;
use serde_json::Number;

use super::PricingInputs;

const CACHE_FIELDS: [&str; 5] = [
    "cache_creation_input_token_cost",
    "cache_creation_input_token_cost_above_1hr",
    "cache_creation_input_token_cost_above_200k_tokens",
    "cache_read_input_token_cost",
    "cache_read_input_token_cost_above_200k_tokens",
];
const SERVICE_TIER_SUFFIXES: [&str; 4] = ["_balanced", "_flex", "_priority", "_ultrafast"];
const BASE_PRICE_FIELDS: [&str; 4] = [
    "input_cost_per_token",
    "input_cost_per_second",
    "cost_per_second",
    "tiered_pricing",
];
const PTU_FIELDS: [&str; 5] = [
    "ptu_count",
    "cost_per_ptu_per_hour",
    "team_id",
    "ptu_effective_from",
    "ptu_effective_to",
];

#[derive(Clone, Copy, Debug)]
pub struct BackendModel<'name> {
    pub model: &'name str,
    pub custom_llm_provider: Option<&'name str>,
}

pub(super) fn is_set(info: &JsonObject, field: &str) -> bool {
    info.get(field)
        .is_some_and(|value| !matches!(value, Json::Null))
}

fn is_zero(number: &Number) -> bool {
    let text = number.to_string();
    text.split(['e', 'E']).next().is_some_and(|mantissa| {
        mantissa
            .chars()
            .all(|digit| matches!(digit, '0' | '.' | '-'))
    })
}

fn has_content(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => !is_zero(number),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

fn scalar_of(value: &Json) -> Scalar {
    match value {
        Json::Null => Scalar::Null,
        Json::Bool(flag) => Scalar::Bool(*flag),
        Json::Number(number) => Scalar::Number(number.clone()),
        Json::Text(text) => Scalar::Text(text.clone()),
        Json::List(items) => Scalar::Array(items.iter().map(|_item| IgnoredAny).collect()),
        Json::Object(_fields) => Scalar::Object(IgnoredAny),
    }
}

fn has_ptu_terms(model_info: &JsonObject) -> bool {
    let scalars: BTreeMap<String, Scalar> = PTU_FIELDS
        .iter()
        .filter_map(|field| {
            model_info
                .get(*field)
                .map(|value| ((*field).to_owned(), scalar_of(value)))
        })
        .collect();
    ptu_terms(&PtuModelInfo::from(&scalars)).is_some()
}

pub struct BackendSource<'run, Lookup> {
    registry: &'run mut ModelRegistry,
    lookup: &'run Lookup,
    backend: BackendModel<'run>,
}

fn raw_backend_entry<'registry>(
    registry: &'registry ModelRegistry,
    info: &JsonObject,
) -> Option<&'registry JsonObject> {
    let Some(Json::Text(key)) = info.get("key") else {
        return None;
    };
    match registry.model_cost.get(key) {
        Some(Json::Object(entry)) => Some(entry),
        _ => None,
    }
}

fn copy_unset_fields<'entry, Fields>(model_info: &mut JsonObject, fields: Fields)
where
    Fields: Iterator<Item = (&'entry String, &'entry Json)>,
{
    for (field, value) in fields {
        if !is_set(model_info, field) && !matches!(value, Json::Null) {
            model_info.insert(field.clone(), value.clone());
        }
    }
}

fn lacks_output_rate(model_info: &JsonObject) -> bool {
    let Some(Json::List(tiers)) = model_info.get("tiered_pricing") else {
        return false;
    };
    let every_tier_prices_output = tiers.iter().all(
        |tier| matches!(tier, Json::Object(fields) if fields.contains_key("output_cost_per_token")),
    );
    !tiers.is_empty() && !is_set(model_info, "output_cost_per_token") && !every_tier_prices_output
}

impl<'run, Lookup: ModelInfoLookup> BackendSource<'run, Lookup> {
    pub const fn new(
        registry: &'run mut ModelRegistry,
        lookup: &'run Lookup,
        backend: BackendModel<'run>,
    ) -> Self {
        Self {
            registry,
            lookup,
            backend,
        }
    }

    // A lookup that finds nothing is LiteLLM's get_model_info raising for an unmapped model, which each step treats as no change.
    async fn info(&mut self) -> Option<JsonObject> {
        let backend = self.backend;
        let name = match backend.custom_llm_provider {
            Some(provider) if !backend.model.starts_with(&format!("{provider}/")) => {
                format!("{provider}/{}", backend.model)
            }
            _ => backend.model.to_owned(),
        };
        let registry = &mut *self.registry;
        self.lookup
            .get_model_info(&name, &registry.model_cost, &mut registry.keys)
            .await
    }

    async fn copy_raw<Keep>(&mut self, model_info: &mut JsonObject, keep: Keep)
    where
        Keep: Fn(&str) -> bool,
    {
        let Some(info) = self.info().await else {
            return;
        };
        let Some(entry) = raw_backend_entry(self.registry, &info) else {
            return;
        };
        copy_unset_fields(
            model_info,
            entry.iter().filter(|(field, _value)| keep(field)),
        );
    }

    pub async fn inherit_cache_pricing(&mut self, model_info: &mut JsonObject) {
        let unset: Vec<&str> = CACHE_FIELDS
            .into_iter()
            .filter(|field| !is_set(model_info, field))
            .collect();
        if unset.is_empty() {
            return;
        }
        let Some(info) = self.info().await else {
            return;
        };
        for field in unset {
            if let Some(value) = info.get(field).filter(|value| !matches!(value, Json::Null)) {
                model_info.insert(field.to_owned(), value.clone());
            }
        }
    }

    pub async fn inherit_service_tier_pricing(
        &mut self,
        model_info: &mut JsonObject,
        inputs: &PricingInputs,
    ) {
        let priced = is_set(model_info, "input_cost_per_token")
            || is_set(model_info, "output_cost_per_token");
        if !priced || (inputs.ptu_cost_attribution && has_ptu_terms(model_info)) {
            return;
        }
        self.copy_raw(model_info, |field| {
            SERVICE_TIER_SUFFIXES
                .iter()
                .any(|tier| field.ends_with(tier))
        })
        .await;
    }

    pub async fn inherit_off_peak_rates(&mut self, model_info: &mut JsonObject) {
        let priced = BASE_PRICE_FIELDS
            .iter()
            .any(|name| is_set(model_info, name));
        if !priced && model_info.get("off_peak_pricing").is_some_and(has_content) {
            self.copy_raw(model_info, |_field| true).await;
        }
    }

    pub async fn inherit_tiered_output_rate(&mut self, model_info: &mut JsonObject) {
        if lacks_output_rate(model_info) {
            let info = self.info().await.unwrap_or_default();
            copy_unset_fields(
                model_info,
                info.iter().filter(|(field, rate)| {
                    field.as_str() == "output_cost_per_token" && has_content(rate)
                }),
            );
        }
    }
}
