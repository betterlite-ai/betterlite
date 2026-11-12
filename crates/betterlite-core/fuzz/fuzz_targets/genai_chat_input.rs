#![no_main]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_core::api::genai_chat_input::input::GenerateContentRequest;
use betterlite_core::api::genai_chat_input::{TranslationContext, translate_generate_content};
use betterlite_core::schema::SchemaLimits;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 0x0004_0000;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > MAX_INPUT {
        return;
    }
    let Ok(request) = serde_json::from_slice::<GenerateContentRequest>(bytes) else {
        return;
    };
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let capabilities = CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    };
    let context = TranslationContext {
        resolution: &resolution,
        model_capabilities: &capabilities,
        limits: SchemaLimits::default(),
    };
    drop(translate_generate_content(&request, None, &context));
});
