use std::future::Future;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::vertex_claude_chat::output::{OUTPUT_CONFIG, sanitize_output_params};

pub const ANTHROPIC_VERSION_FIELD: &str = "anthropic_version";
pub const VERTEX_ANTHROPIC_VERSION: &str = "vertex-2023-10-16";
pub const MODEL_FIELD: &str = "model";

// B1333 owns the output-parameter policy; the native path only has to run it on the request it built.
pub trait OutputSanitizer {
    fn sanitize<'body>(
        &'body mut self,
        body: &'body mut IndexMap<String, ContractJson>,
        model: &'body str,
    ) -> impl Future<Output = ()> + Send + 'body;
}

pub struct ClaudeSanitizer<'environment, 'resources, Source, Transport> {
    pub environment: &'environment mut CapabilityEnvironment<'resources, Source, Transport>,
    pub catalogs: &'environment CatalogSources<'resources>,
}

impl<Source, Transport> OutputSanitizer for ClaudeSanitizer<'_, '_, Source, Transport>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    fn sanitize<'body>(
        &'body mut self,
        body: &'body mut IndexMap<String, ContractJson>,
        model: &'body str,
    ) -> impl Future<Output = ()> + Send + 'body {
        sanitize_output_params(body, model, self.environment, self.catalogs)
    }
}

// LiteLLM's Vertex body overrides after the parent normalization: the version moves into the body, the model name leaves it, and an absent output_config has nothing to sanitize.
pub async fn apply<Sanitize>(
    body: &mut Object,
    model: &str,
    sanitizer: &mut Sanitize,
) -> Result<(), BodyError>
where
    Sanitize: OutputSanitizer + Send,
{
    body.insert(
        ANTHROPIC_VERSION_FIELD.to_owned(),
        Json::Text(VERTEX_ANTHROPIC_VERSION.to_owned()),
    );
    body.shift_remove(MODEL_FIELD);
    if !body.contains_key(OUTPUT_CONFIG) {
        return Ok(());
    }
    let mut contract: IndexMap<String, ContractJson> = convert(body)?;
    sanitizer.sanitize(&mut contract, model).await;
    *body = convert(&contract)?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
#[error("the request body does not convert between the JSON shapes: {0}")]
pub struct BodyError(#[from] serde_json::Error);

// betterlite-models and betterlite-prompts each declare the same untagged JSON enum, so serde carries the body between them, as the native transform's own bridge does.
fn convert<From, To>(value: &From) -> Result<To, BodyError>
where
    From: Serialize + ?Sized,
    To: DeserializeOwned,
{
    serde_json::to_string(value)
        .and_then(|encoded| serde_json::from_str(&encoded))
        .map_err(BodyError::from)
}
