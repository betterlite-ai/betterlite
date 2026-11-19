use std::future::Future;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;

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
pub async fn apply<Sanitize>(body: &mut Object, model: &str, sanitizer: &mut Sanitize)
where
    Sanitize: OutputSanitizer + Send,
{
    body.insert(
        ANTHROPIC_VERSION_FIELD.to_owned(),
        Json::Text(VERTEX_ANTHROPIC_VERSION.to_owned()),
    );
    body.shift_remove(MODEL_FIELD);
    if !body.contains_key(OUTPUT_CONFIG) {
        return;
    }
    let mut contract = to_contract_object(body);
    sanitizer.sanitize(&mut contract, model).await;
    *body = from_contract_object(&contract);
}

// betterlite-models and betterlite-prompts each declare the same untagged JSON enum, and B1333's policy takes the former.
fn to_contract_object(object: &Object) -> IndexMap<String, ContractJson> {
    object
        .iter()
        .map(|(key, value)| (key.clone(), to_contract_json(value)))
        .collect()
}

fn to_contract_json(value: &Json) -> ContractJson {
    match value {
        Json::Null => ContractJson::Null,
        Json::Bool(flag) => ContractJson::Bool(*flag),
        Json::Number(number) => ContractJson::Number(number.clone()),
        Json::Text(text) => ContractJson::Text(text.clone()),
        Json::List(items) => ContractJson::List(items.iter().map(to_contract_json).collect()),
        Json::Object(fields) => ContractJson::Object(to_contract_object(fields)),
    }
}

fn from_contract_object(object: &IndexMap<String, ContractJson>) -> Object {
    object
        .iter()
        .map(|(key, value)| (key.clone(), from_contract_json(value)))
        .collect()
}

fn from_contract_json(value: &ContractJson) -> Json {
    match value {
        ContractJson::Null => Json::Null,
        ContractJson::Bool(flag) => Json::Bool(*flag),
        ContractJson::Number(number) => Json::Number(number.clone()),
        ContractJson::Text(text) => Json::Text(text.clone()),
        ContractJson::List(items) => Json::List(items.iter().map(from_contract_json).collect()),
        ContractJson::Object(fields) => Json::Object(from_contract_object(fields)),
    }
}
