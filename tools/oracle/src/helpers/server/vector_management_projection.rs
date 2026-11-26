use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::api::vector_management_boundaries::ownership::with_ownership;
use betterlite_server::api::vector_management_boundaries::redaction::redact_litellm_params;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    OwnershipProjection {
        // the box keeps the input enum's largest variant small enough for clippy large_enum_variant
        vector_store: Box<LiteLlmManagedVectorStore>,
    },
    RedactVectorCredentials {
        litellm_params: serde_json::Value,
        depth: usize,
    },
    SerializedVectorParams {
        litellm_params: String,
        depth: usize,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::OwnershipProjection { vector_store } => {
                Ok(serde_json::to_value(with_ownership(vector_store))?)
            }
            Input::RedactVectorCredentials {
                litellm_params,
                depth,
            } => redact(litellm_params, *depth),
            Input::SerializedVectorParams {
                litellm_params,
                depth,
            } => redact(&serde_json::Value::String(litellm_params.clone()), *depth),
        }
    }
}

fn redact(litellm_params: &serde_json::Value, depth: usize) -> HelperResult {
    let parsed: Json = serde_json::from_value(litellm_params.clone())?;
    let redacted = redact_litellm_params(&parsed, depth, ApiLimits::default());
    Ok(serde_json::to_value(redacted)?)
}
