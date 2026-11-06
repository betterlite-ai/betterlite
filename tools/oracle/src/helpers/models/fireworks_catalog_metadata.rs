use std::sync::Arc;

use betterlite_models::entry::ModelFields;
use betterlite_models::fireworks_catalog_metadata::{
    Capability, Catalog, FireworksMetadata, MODEL_COST_KEY_PREFIX, capability_exact,
    resolve_resource_name,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    catalog: Vec<CatalogModel>,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogModel {
    key: String,
    model_info: ModelFields,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    ResolveResource {
        model: String,
    },
    CostKey {
        model: String,
    },
    ExactCapability {
        model: String,
        capability: Capability,
    },
    Capability {
        model: String,
        capability: Capability,
    },
    ProviderInfo {
        model: String,
    },
    Generation,
    Add {
        key: String,
        model_info: Box<ModelFields>,
    },
    Remove {
        key: String,
    },
    Invalidate,
    ReplaceCatalog {
        catalog: Box<[CatalogModel]>,
    },
}

pub(crate) struct Op;

struct Session {
    catalog: Arc<Catalog>,
    retired: Vec<Arc<Catalog>>,
    metadata: FireworksMetadata,
    generation: u64,
}

fn catalog_of(entries: &[CatalogModel]) -> Catalog {
    entries
        .iter()
        .map(|entry| (entry.key.clone(), entry.model_info.clone()))
        .collect()
}

fn helper_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "SerializationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

impl Session {
    fn apply(&mut self, step: &Step) -> Result<Value, Box<HelperError>> {
        match step {
            Step::ResolveResource { model } => Ok(json!(resolve_resource_name(model))),
            Step::CostKey { model } => Ok(json!(format!(
                "{MODEL_COST_KEY_PREFIX}{}",
                resolve_resource_name(model)
            ))),
            Step::ExactCapability { model, capability } => Ok(json!(capability_exact(
                self.catalog.as_ref(),
                model,
                *capability
            ))),
            Step::Capability { model, capability } => {
                Ok(json!(self.metadata.get_model_cost_capability(
                    self.catalog.as_ref(),
                    self.generation,
                    model,
                    *capability
                )))
            }
            Step::ProviderInfo { model } => self.provider_info(model),
            Step::Generation => Ok(json!(self.generation)),
            Step::Add { key, model_info } => {
                Arc::make_mut(&mut self.catalog).insert(key.clone(), model_info.as_ref().clone());
                Ok(Value::Null)
            }
            Step::Remove { key } => {
                Arc::make_mut(&mut self.catalog).shift_remove(key);
                Ok(Value::Null)
            }
            Step::Invalidate => {
                self.generation = self.generation.saturating_add(1);
                Ok(json!(self.generation))
            }
            Step::ReplaceCatalog { catalog } => {
                let next = Arc::new(catalog_of(catalog));
                let previous = std::mem::replace(&mut self.catalog, next);
                self.retired.push(previous);
                Ok(Value::Null)
            }
        }
    }

    fn provider_info(&mut self, model: &str) -> Result<Value, Box<HelperError>> {
        let info = self
            .metadata
            .get_provider_info(self.catalog.as_ref(), self.generation, model);
        serde_json::to_value(info).map_err(|error| helper_error(error.to_string()))
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut session = Session {
            catalog: Arc::new(catalog_of(&input.catalog)),
            retired: Vec::new(),
            metadata: FireworksMetadata::default(),
            generation: 0,
        };
        let mut results = Vec::new();
        for step in &input.steps {
            results.push(session.apply(step)?);
        }
        Ok(Value::Array(results))
    }
}

mod tests;
