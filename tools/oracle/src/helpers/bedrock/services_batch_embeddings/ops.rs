use betterlite_bedrock::batch_embeddings::classify::classify_batch_record;
use betterlite_bedrock::batch_embeddings::error::BatchEmbeddingError;
use betterlite_bedrock::batch_embeddings::input::coerce_input_to_string;
use betterlite_bedrock::batch_embeddings::mapping::map_embedding_to_bedrock;
use betterlite_bedrock::batch_embeddings::titan::{
    is_titan_v2_embed_model, lookup_provider_specific_field,
};
use betterlite_bedrock::model_routes::ParamMap;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::{
    ClassifyRecords, CoerceInput, Input, MapEmbedding, NameBoundary, RegistrySchema,
};
use super::registry::{Registry, RegistryError};

const INVOCATION_SCHEMA_FIELD: &str = "bedrock_invocation_schema";

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::ClassifyRecords(case) => Ok(classify_records(case)),
        Input::RegistrySchema(case) => registry_schema(case),
        Input::NameBoundary(case) => name_boundary(case),
        Input::CoerceInput(case) => coerce_input(case),
        Input::MapEmbedding(case) => map_embedding(case),
    }
}

fn classify_records(case: &ClassifyRecords) -> Value {
    let kinds: Vec<Value> = case
        .records
        .iter()
        .map(|record| json!(classify_batch_record(record)))
        .collect();
    json!({ "kinds": kinds })
}

// one schema and the flag it drives per id, so the override and the fallback stay side by side
fn registry_schema(case: &RegistrySchema) -> HelperResult {
    let registry = Registry::load(&case.model_cost).map_err(Failure::from)?;
    let runtime = Builder::new_current_thread().build()?;
    let mut results = Vec::with_capacity(case.models.len());
    for model in &case.models {
        let schema = runtime.block_on(lookup_provider_specific_field(
            model,
            INVOCATION_SCHEMA_FIELD,
            &registry,
        ));
        let titan = runtime.block_on(is_titan_v2_embed_model(model, &registry));
        results.push(json!({ "schema": schema, "titan": titan }));
    }
    Ok(json!({ "results": results }))
}

// no model_cost at all, so every lookup misses and every answer comes from the id alone
fn name_boundary(case: &NameBoundary) -> HelperResult {
    let registry = Registry::load(&ParamMap::new()).map_err(Failure::from)?;
    let runtime = Builder::new_current_thread().build()?;
    let mut results = Vec::with_capacity(case.models.len());
    for model in &case.models {
        let titan = runtime.block_on(is_titan_v2_embed_model(model, &registry));
        results.push(json!(titan));
    }
    Ok(json!({ "results": results }))
}

fn coerce_input(case: &CoerceInput) -> HelperResult {
    let coerced =
        coerce_input_to_string(case.input.as_ref(), &case.model).map_err(Failure::from)?;
    Ok(json!({ "coerced": coerced }))
}

fn map_embedding(case: &MapEmbedding) -> HelperResult {
    let registry = Registry::load(&case.model_cost).map_err(Failure::from)?;
    let runtime = Builder::new_current_thread().build()?;
    let mapped = runtime
        .block_on(map_embedding_to_bedrock(&case.body, &case.model, &registry))
        .map_err(Failure::from)?;
    Ok(serde_json::to_value(mapped)?)
}

#[derive(Debug, Error)]
enum Failure {
    #[error(transparent)]
    Embedding(#[from] BatchEmbeddingError),
    #[error(transparent)]
    Registry(#[from] RegistryError),
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        Self::new(HelperError {
            class: error_class(&failure).to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}

// the class is stripped before the comparison, so it only names the Python failure a reader would see
const fn error_class(failure: &Failure) -> &'static str {
    match failure {
        Failure::Embedding(error) => embedding_class(error),
        Failure::Registry(_) => "ValueError",
    }
}

const fn embedding_class(error: &BatchEmbeddingError) -> &'static str {
    match error {
        BatchEmbeddingError::MissingInput { .. }
        | BatchEmbeddingError::MultipleInputs { .. }
        | BatchEmbeddingError::InputNotString { .. } => "ValueError",
        BatchEmbeddingError::TokenizedInput { .. }
        | BatchEmbeddingError::UnsupportedModel { .. } => "NotImplementedError",
        BatchEmbeddingError::Titan(_) => "TypeError",
    }
}
