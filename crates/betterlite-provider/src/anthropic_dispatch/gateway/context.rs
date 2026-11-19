use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_core::cost::completion::lookup::NoHfConfig;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::MetadataEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::model_metadata_resolution::names::NameNormalizers;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::resolution::ResolutionContext;

use crate::anthropic_dispatch::completion::CompletionSettings;
use crate::anthropic_dispatch::gateway::runtime::AnthropicRuntime;

pub(super) fn settings(runtime: &AnthropicRuntime) -> CompletionSettings<'_> {
    CompletionSettings {
        globals: &runtime.bases.globals,
        environment: &runtime.bases.environment,
        custom_prompt_dict: &runtime.custom_prompts,
        disable_suffix: runtime.bases.disable_suffix,
        // The messages surface is always the provider's own config; selected_config needs it named.
        selection: Some(ChatConfigTag::Anthropic),
    }
}

pub(super) fn catalogs(runtime: &AnthropicRuntime) -> CatalogSources<'_> {
    CatalogSources {
        patterns: &runtime.patterns,
        runtime: &runtime.source,
        packaged: &runtime.source,
    }
}

pub(super) fn capability_environment<'call>(
    runtime: &'call AnthropicRuntime,
    resolution: &'call ResolutionContext<'call>,
    keys: &'call mut ModelCostKeys,
) -> CapabilityEnvironment<'call, NoDynamicMetadata, NoHfConfig> {
    CapabilityEnvironment {
        metadata: metadata(runtime, resolution),
        keys,
        dynamic: &NoDynamicMetadata,
        hf: &runtime.hf,
        provider_catalog: &runtime.provider_catalog,
        provider_generation: 0,
    }
}

pub(super) fn resolution(runtime: &AnthropicRuntime) -> ResolutionContext<'_> {
    ResolutionContext {
        catalog: &runtime.catalog,
        known_model_names: &runtime.known_model_names,
        custom_providers: &runtime.custom_providers,
        environment: &runtime.environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: runtime.credentials.clone(),
    }
}

fn metadata<'call>(
    runtime: &'call AnthropicRuntime,
    resolution: &'call ResolutionContext<'call>,
) -> MetadataEnvironment<'call> {
    MetadataEnvironment {
        model_cost: &runtime.map.entries,
        vertex: &runtime.map.vertex,
        generalizations: &runtime.catalog.generalizations,
        resolution,
        normalizers: NameNormalizers {
            bedrock_base_model: get_bedrock_base_model,
            bedrock_routing_prefix: strip_bedrock_routing_prefix,
            mantle_region_split: betterlite_bedrock::mantle_inputs::region::split_region_prefix,
        },
    }
}
