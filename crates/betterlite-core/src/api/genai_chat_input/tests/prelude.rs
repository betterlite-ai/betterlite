use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::catalog::BundledCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::chat::{ChatMessage, ChatRequest, MessageContent};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use serde_json::Value;

use crate::api::genai_chat_input::{
    GenAiInputError, GenerateContentRequest, GenerateLiteLLMParams, TranslatedCompletion,
    TranslationContext, translate_generate_content,
};
use crate::schema::SchemaLimits;

pub(super) fn limits_with_nodes(max_nodes: usize) -> SchemaLimits {
    let defaults = SchemaLimits::default();
    SchemaLimits {
        max_schema_bytes: defaults.max_schema_bytes,
        max_response_bytes: defaults.max_response_bytes,
        max_depth: defaults.max_depth,
        max_nodes,
    }
}

pub(super) fn request(body: Value) -> GenerateContentRequest {
    serde_json::from_value(body).expect("a GenAI request")
}

pub(super) fn translate(body: Value) -> ChatRequest {
    translate_result(body, None, SchemaLimits::default())
        .expect("the translated request")
        .0
}

pub(super) fn translate_with(body: Value, params: &GenerateLiteLLMParams) -> TranslatedCompletion {
    translate_result(body, Some(params), SchemaLimits::default()).expect("the translated request")
}

pub(super) fn translate_result(
    body: Value,
    params: Option<&GenerateLiteLLMParams>,
    limits: SchemaLimits,
) -> Result<TranslatedCompletion, GenAiInputError> {
    translate_request(&request(body), params, limits)
}

pub(super) fn translate_request(
    request: &GenerateContentRequest,
    params: Option<&GenerateLiteLLMParams>,
    limits: SchemaLimits,
) -> Result<TranslatedCompletion, GenAiInputError> {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let names = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: catalog.as_ref(),
        known_model_names: &names,
        custom_providers: &names,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let capabilities = CatalogCapabilities {
        chat_models: BTreeSet::from([
            String::from("gpt-4"),
            String::from("gpt-4o"),
            String::from("o3"),
        ]),
        text_models: BTreeSet::new(),
    };
    let context = TranslationContext {
        resolution: &resolution,
        model_capabilities: &capabilities,
        limits,
    };
    translate_generate_content(request, params, &context)
}

pub(super) fn text_of(message: &ChatMessage) -> String {
    match message.content.as_ref().expect("message content") {
        MessageContent::Text(text) => text.clone(),
        MessageContent::Parts(parts) => parts
            .first()
            .and_then(|part| part.fields.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    }
}
