use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};

use betterlite_bedrock::mantle_inputs::region::split_region_prefix;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::entry::ModelFields;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_models::model_capability_flags::{self, ModelCapabilityFlag};
use betterlite_models::model_capability_predicates::{CapabilityEnvironment, CapabilityRequest};
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::MetadataEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::{
    DynamicMetadataSource, NoDynamicMetadata,
};
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_models::model_metadata_resolution::names::{ModelCostMap, NameNormalizers};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::resolution::ResolutionContext;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use serde_json::value::RawValue;

use crate::helpers::{HelperError, HelperResult};

pub(crate) struct Op;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    function: String,
    model: String,
    #[serde(default)]
    custom_llm_provider: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    fixture: FixtureInput,
    steps: Vec<Step>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureInput {
    #[serde(default)]
    model_cost: IndexMap<String, Box<RawValue>>,
}

// The sets and flags cases name no huggingface model, so the transport refuses instead of reaching the network.
struct NoHfTransport;

#[derive(Debug, thiserror::Error)]
#[error("the metadata lookup must not fetch a Hugging Face config")]
struct NoHfError;

impl ModelInfoClient for NoHfTransport {
    type Error = NoHfError;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, NoHfError>> + Send {
        ready(Err(NoHfError))
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let bytes = serde_json::to_vec(&input.fixture.model_cost)
            .map_err(|source| serialization(&source))?;
        let (sets, by_provider) = sets_answer(&bytes)?;
        let flags = flags_answer(&bytes, &input.steps)?;
        Ok(json!({"sets": sets, "by_provider": by_provider, "flags": flags}))
    }
}

fn sets_answer(bytes: &[u8]) -> Result<(Value, Value), Box<HelperError>> {
    let catalog =
        ModelCatalog::parse(bytes, UnknownFields::Warn).map_err(|source| serialization(&source))?;
    let mut sets = ProviderModelSets::default();
    sets.add_known_models(&catalog);
    let mut reported = serde_json::Map::new();
    for (set, names) in &sets.sets {
        reported.insert(set.to_string(), json!(names));
    }
    let by_provider =
        serde_json::to_value(&sets.models_by_provider).map_err(|source| serialization(&source))?;
    Ok((Value::Object(reported), by_provider))
}

fn fixture_inputs(bytes: &[u8]) -> Result<(ModelCostMap, ModelCatalog, Catalog), Box<HelperError>> {
    let cost_map = ModelCostMap::parse(bytes).map_err(|source| serialization(&source))?;
    let catalog =
        ModelCatalog::parse(bytes, UnknownFields::Warn).map_err(|source| serialization(&source))?;
    let entries: IndexMap<String, Box<RawValue>> =
        serde_json::from_slice(bytes).map_err(|source| serialization(&source))?;
    let mut provider_catalog = Catalog::new();
    for (name, entry) in &entries {
        if let Ok((fields, _warnings)) = ModelFields::parse(entry) {
            provider_catalog.insert(name.clone(), fields);
        }
    }
    Ok((cost_map, catalog, provider_catalog))
}

fn flags_answer(bytes: &[u8], steps: &[Step]) -> Result<Value, Box<HelperError>> {
    let (cost_map, catalog, provider_catalog) = fixture_inputs(bytes)?;
    let generalizations = CatalogGeneralizations::default();
    let mut keys = ModelCostKeys::default();
    let dynamic = NoDynamicMetadata;
    let known_model_names = BTreeSet::new();
    let custom_providers = BTreeSet::new();
    let environment_vars = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known_model_names,
        custom_providers: &custom_providers,
        environment: &environment_vars,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let hf = HfConfigClient {
        client: NoHfTransport,
        base_url: HF_CONFIG_BASE_URL.to_owned(),
    };
    let mut environment = CapabilityEnvironment {
        metadata: MetadataEnvironment {
            model_cost: &cost_map.entries,
            vertex: &cost_map.vertex,
            generalizations: &generalizations,
            resolution: &resolution,
            normalizers: NORMALIZERS,
        },
        keys: &mut keys,
        dynamic: &dynamic,
        hf: &hf,
        provider_catalog: &provider_catalog,
        provider_generation: 0,
    };
    run_steps(steps, &mut environment)
}

fn run_steps<Source, Transport>(
    steps: &[Step],
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<Value, Box<HelperError>>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| HelperError {
            class: String::from("RuntimeError"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: source.to_string(),
        })?;
    runtime.block_on(async {
        let mut flags = Vec::with_capacity(steps.len());
        for step in steps {
            flags.push(Value::Bool(answer(step, environment).await?));
        }
        Ok(Value::Array(flags))
    })
}

// The eleven named wrappers, one shared-factory call each, plus the opt-out reader.
const FUNCTIONS: [(&str, ModelCapabilityFlag); 11] = [
    ("supports_web_search", ModelCapabilityFlag::WebSearch),
    ("supports_url_context", ModelCapabilityFlag::UrlContext),
    ("supports_audio_input", ModelCapabilityFlag::AudioInput),
    ("supports_pdf_input", ModelCapabilityFlag::PdfInput),
    ("supports_audio_output", ModelCapabilityFlag::AudioOutput),
    (
        "supports_prompt_cache_breakpoint",
        ModelCapabilityFlag::PromptCacheBreakpoint,
    ),
    (
        "supports_thinking_cache_preservation",
        ModelCapabilityFlag::ThinkingCachePreservation,
    ),
    ("supports_computer_use", ModelCapabilityFlag::ComputerUse),
    ("supports_vision", ModelCapabilityFlag::Vision),
    (
        "supports_anthropic_thinking_payload",
        ModelCapabilityFlag::AnthropicThinkingPayload,
    ),
    (
        "supports_embedding_image_input",
        ModelCapabilityFlag::EmbeddingImageInput,
    ),
];

async fn answer<Source, Transport>(
    step: &Step,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
) -> Result<bool, Box<HelperError>>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let model = step.model.as_str();
    let provider = step.custom_llm_provider.as_deref();
    if step.function == "is_vision_explicitly_disabled" {
        let request = CapabilityRequest {
            model,
            custom_llm_provider: provider,
        };
        return Ok(
            model_capability_flags::is_vision_explicitly_disabled(environment, &request).await,
        );
    }
    let Some((_, flag)) = FUNCTIONS
        .iter()
        .find(|(function, _)| *function == step.function.as_str())
    else {
        return Err(unknown_function(&step.function));
    };
    Ok(model_capability_flags::supports(environment, model, provider, *flag).await)
}

fn unknown_function(function: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: format!("unknown capability function: {function}"),
    })
}

fn serialization<Source>(source: &Source) -> Box<HelperError>
where
    Source: std::fmt::Display,
{
    Box::new(HelperError {
        class: String::from("SerializationError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: source.to_string(),
    })
}
