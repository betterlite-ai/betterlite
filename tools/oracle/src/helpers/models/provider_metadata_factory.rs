use std::collections::BTreeMap;

use betterlite_models::entry::ModelFields;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::provider_metadata_factory::adapters::{FireworksSettings, LemonadeSettings};
use betterlite_models::provider_metadata_factory::selection::{
    MetadataQuery, ProviderModelMetadata, get_provider_info, select,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    MetadataAdapterSelection(SelectionInput),
    ProviderInfoProjection(ProjectionInput),
    MetadataConstructors(ConstructorsInput),
    DynamicMetadataSeparation(DynamicInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SelectionInput {
    queries: Vec<SelectionQuery>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionQuery {
    #[serde(default)]
    model: Option<String>,
    providers: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectionInput {
    #[serde(default)]
    catalog: Vec<CatalogModel>,
    steps: Vec<ProjectionStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogModel {
    key: String,
    model_info: ModelFields,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum ProjectionStep {
    Free {
        model: String,
        #[serde(default)]
        provider: Option<String>,
    },
    AdapterProviderInfo {
        model: String,
        provider: String,
    },
    AdapterCostKey {
        model: String,
        provider: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConstructorsInput {
    #[serde(default)]
    fireworks: Vec<FireworksSettings>,
    #[serde(default)]
    lemonade: Vec<LemonadeSettings>,
    #[serde(default)]
    foundry: Vec<Option<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DynamicInput {
    queries: Vec<DynamicQuery>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DynamicQuery {
    provider: String,
    model: String,
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn provider_id(name: &str) -> Result<ProviderId, Box<HelperError>> {
    name.parse::<ProviderId>()
        .map_err(|error| helper_error("ValueError", error.to_string()))
}

fn to_json<T>(value: T) -> HelperResult
where
    T: serde::Serialize,
{
    serde_json::to_value(value)
        .map_err(|error| helper_error("SerializationError", error.to_string()))
}

fn selection_entry(provider: &str, selected: &ProviderModelMetadata) -> Value {
    if provider == "azure_ai" {
        let model = if let ProviderModelMetadata::AzureAi(config) = selected {
            config.model.as_deref()
        } else {
            None
        };
        json!({"class": selected.class_name(), "model": model})
    } else {
        json!({"class": selected.class_name()})
    }
}

fn selection(input: &SelectionInput) -> HelperResult {
    let mut observations = Vec::new();
    for query in &input.queries {
        let mut chosen: BTreeMap<String, Value> = BTreeMap::new();
        for provider in &query.providers {
            let selected = select(query.model.as_deref(), provider_id(provider)?);
            let entry = selected.as_ref().map_or_else(
                || json!({"class": null}),
                |adapter| selection_entry(provider, adapter),
            );
            chosen.insert(provider.clone(), entry);
        }
        observations.push(Value::Object(chosen.into_iter().collect()));
    }
    Ok(Value::Array(observations))
}

fn catalog_of(entries: &[CatalogModel]) -> Catalog {
    entries
        .iter()
        .map(|entry| (entry.key.clone(), entry.model_info.clone()))
        .collect()
}

fn projection_step(step: &ProjectionStep, catalog: &Catalog) -> Result<Value, Box<HelperError>> {
    let query = MetadataQuery {
        catalog,
        generation: 0,
    };
    match step {
        ProjectionStep::Free { model, provider } => {
            to_json(get_provider_info(model, provider.as_deref(), query))
        }
        ProjectionStep::AdapterProviderInfo { model, provider } => {
            let mut selected = select(Some(model), provider_id(provider)?);
            selected.as_mut().map_or(Ok(Value::Null), |adapter| {
                to_json(adapter.get_provider_info(model, query))
            })
        }
        ProjectionStep::AdapterCostKey { model, provider } => {
            let selected = select(Some(model), provider_id(provider)?);
            selected.map_or(Ok(Value::Null), |adapter| {
                to_json(adapter.get_model_cost_key(model))
            })
        }
    }
}

fn projection(input: &ProjectionInput) -> HelperResult {
    let catalog = catalog_of(&input.catalog);
    let mut results = Vec::new();
    for step in &input.steps {
        results.push(projection_step(step, &catalog)?);
    }
    Ok(Value::Array(results))
}

fn constructors(input: &ConstructorsInput) -> HelperResult {
    let mut fireworks = Vec::new();
    let mut fireworks_settings = FireworksSettings::default();
    for args in &input.fireworks {
        fireworks_settings = fireworks_settings.with_args(args.clone());
        fireworks.push(to_json(&fireworks_settings)?);
    }
    let mut lemonade = Vec::new();
    let mut lemonade_settings = LemonadeSettings::default();
    for args in &input.lemonade {
        lemonade_settings = lemonade_settings.with_args(args.clone());
        lemonade.push(to_json(&lemonade_settings)?);
    }
    let foundry: Vec<Value> = input
        .foundry
        .iter()
        .map(|model| {
            let stored =
                select(model.as_deref(), ProviderId::AzureAi).and_then(|adapter| match adapter {
                    ProviderModelMetadata::AzureAi(config) => config.model,
                    ProviderModelMetadata::FireworksAi(_)
                    | ProviderModelMetadata::Xai
                    | ProviderModelMetadata::Ollama
                    | ProviderModelMetadata::Lemonade(_)
                    | ProviderModelMetadata::Stateless(_) => None,
                });
            json!({"model": stored})
        })
        .collect();
    Ok(json!({"fireworks": fireworks, "lemonade": lemonade, "foundry": foundry}))
}

fn separation(input: &DynamicInput) -> HelperResult {
    let mut results = Vec::new();
    for query in &input.queries {
        let selected = select(Some(&query.model), provider_id(&query.provider)?);
        let entry = selected.map_or_else(
            || {
                json!({
                    "provider": query.provider,
                    "class": null,
                    "dynamic": null,
                    "calls": 0_u8,
                })
            },
            |adapter| {
                json!({
                    "provider": query.provider,
                    "class": adapter.class_name(),
                    "dynamic": adapter.dynamic_model_info(),
                    "calls": 0_u8,
                })
            },
        );
        results.push(entry);
    }
    Ok(Value::Array(results))
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::MetadataAdapterSelection(value) => selection(value),
            Input::ProviderInfoProjection(value) => projection(value),
            Input::MetadataConstructors(value) => constructors(value),
            Input::DynamicMetadataSeparation(value) => separation(value),
        }
    }
}
