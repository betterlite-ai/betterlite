use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_config::sections::anthropic_capability_catalog::Settings as Budgets;
use betterlite_config::sections::anthropic_completion::OptionValues;
use betterlite_config::tls::Verification;
use betterlite_config::types::Config;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::anthropic_catalog_lookup::error::CatalogLookupError;
use betterlite_context::local_ttl_cache::CacheError;
use betterlite_core::api::anthropic_beta_config::BetaConfigLoader;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::cost::completion::lookup::NoHfConfig;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use betterlite_models::catalog::{BundledCatalog, CatalogError, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_models::model_metadata_resolution::names::{ModelCostMap, ModelCostMapError};
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::Hub;
use betterlite_prompts::media_fetch::fetcher::Fetcher;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets::bundled;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use secrecy::ExposeSecret as _;
use uuid::Uuid;

use crate::anthropic_dispatch::base::{Environment, Globals};
use crate::anthropic_dispatch::gateway::logging::TracingStreamLogger;

pub struct AnthropicBases {
    pub globals: Globals,
    pub environment: Environment,
    pub disable_suffix: bool,
}

impl AnthropicBases {
    #[must_use]
    pub fn new(config: &Config) -> Self {
        let sdk = &config.sections.resource_sdk_settings;
        let credentials = &config.sections.anthropic_credentials;
        Self {
            globals: Globals {
                api_base: sdk.api_base.clone(),
                anthropic_key: None,
                api_key: sdk.api_key.clone(),
            },
            environment: Environment {
                api_base: Some(credentials.api_base.clone()),
                base_url: None,
                api_key: credentials.api_key.clone(),
            },
            disable_suffix: credentials.disable_url_suffix,
        }
    }
}

pub struct AnthropicRuntime {
    pub bases: AnthropicBases,
    pub settings: AnthropicSettings,
    pub beta: BetaConfigLoader,
    pub catalog: Arc<ModelCatalog>,
    pub map: ModelCostMap,
    pub patterns: CandidatePatterns,
    pub source: RawCatalog,
    pub provider_catalog: Catalog,
    pub environment: BTreeMap<String, String>,
    pub known_model_names: BTreeSet<String>,
    pub custom_providers: BTreeSet<String>,
    pub credentials: GlobalCredentials,
    pub fetcher: Fetcher<SystemResolver>,
    pub defaults: OptionDefaults,
    pub legacy_defaults: OptionValues,
    pub budgets: Budgets,
    pub max_tokens: i64,
    pub global_drop_params: bool,
    pub modify_params: bool,
    pub disable_add_prefix_to_prompt: bool,
    pub redactor: Redactor,
    pub engine: Engine,
    pub new_id: fn() -> String,
    pub custom_prompts: BTreeMap<String, Json>,
    pub prompt_factory: PromptFactory,
    pub hf: HfConfigClient<NoHfConfig>,
    pub stream_logger: TracingStreamLogger,
    pub max_event_bytes: usize,
    pub max_error_body_bytes: usize,
}

impl fmt::Debug for AnthropicRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AnthropicRuntime")
            .finish_non_exhaustive()
    }
}

impl AnthropicRuntime {
    pub fn new(
        config: &Config,
        client: &ProxiedClient,
        catalog: Arc<ModelCatalog>,
    ) -> Result<Self, RuntimeError> {
        let (map, source, patterns) = bundled_catalogs()?;
        let options = &config.sections.anthropic_option_primitives;
        Ok(Self {
            bases: AnthropicBases::new(config),
            settings: settings(config),
            beta: beta_config(config, client),
            catalog,
            map,
            patterns,
            source,
            provider_catalog: Catalog::new(),
            environment: environment(config),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            credentials: credentials(config),
            fetcher: fetcher(config, client)?,
            defaults: option_defaults(&options.litellm_settings),
            legacy_defaults: config
                .sections
                .anthropic_completion
                .litellm_settings
                .clone(),
            budgets: config.sections.anthropic_capability_catalog,
            max_tokens: options.default_chat_max_tokens,
            global_drop_params: config.sections.drop_params.policy.global_drop_params,
            modify_params: config.litellm_settings.modify_params,
            disable_add_prefix_to_prompt: config
                .sections
                .anthropic_response
                .inner
                .global_disable_prefix,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?,
            engine: engine()?,
            new_id: || Uuid::new_v4().to_string(),
            custom_prompts: BTreeMap::new(),
            prompt_factory: prompt_factory(),
            hf: hf(),
            stream_logger: TracingStreamLogger,
            max_event_bytes: ApiLimits::default().max_event_bytes,
            max_error_body_bytes: client.limits.body_bytes,
        })
    }
}

fn beta_config(config: &Config, client: &ProxiedClient) -> BetaConfigLoader {
    BetaConfigLoader::new(
        client.clone(),
        config.sections.anthropic_beta_config.clone(),
        ApiLimits::default(),
    )
}

fn credentials(config: &Config) -> GlobalCredentials {
    let sdk = &config.sections.resource_sdk_settings;
    GlobalCredentials {
        api_key: sdk.api_key.clone(),
        api_base: sdk.api_base.clone(),
        xai_key: None,
        inception_key: None,
        lemonade_key: None,
    }
}

// The Messages surface never reads the legacy prompt factory, so its hub never resolves.
fn prompt_factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(BTreeMap::new()),
        hub: Hub::Unreachable,
    }
}

fn hf() -> HfConfigClient<NoHfConfig> {
    HfConfigClient {
        client: NoHfConfig,
        base_url: HF_CONFIG_BASE_URL.to_owned(),
    }
}

fn settings(config: &Config) -> AnthropicSettings {
    let credentials = &config.sections.anthropic_credentials;
    AnthropicSettings {
        api_key: credentials.api_key.clone(),
        auth_token: credentials.auth_token.clone(),
        api_base: credentials.api_base.clone(),
        use_bearer_for_custom_base: credentials.use_bearer_for_custom_base,
        anthropic_version: AnthropicSettings::default().anthropic_version,
    }
}

fn bundled_catalogs() -> Result<(ModelCostMap, RawCatalog, CandidatePatterns), RuntimeError> {
    let bytes = (BundledCatalog::default().asset)()?;
    let source = serde_json::from_slice(&bytes)?;
    let patterns = CandidatePatterns::new()?;
    Ok((ModelCostMap::parse(&bytes)?, source, patterns))
}

fn engine() -> Result<Engine, RuntimeError> {
    let encoding = Encoding::Cl100kBase;
    let vocabulary = bundled(encoding).ok_or(RuntimeError::MissingVocabulary)?;
    let engine = TiktokenEngine::from_bpe_file(encoding, &vocabulary)?;
    Ok(Engine::Tiktoken(Arc::new(engine)))
}

fn environment(config: &Config) -> BTreeMap<String, String> {
    config
        .environment_variables
        .iter()
        .filter_map(|(name, value)| {
            value
                .as_ref()
                .map(|secret| (name.clone(), secret.expose_secret().to_owned()))
        })
        .collect()
}

fn fetcher(
    config: &Config,
    client: &ProxiedClient,
) -> Result<Fetcher<SystemResolver>, RuntimeError> {
    let policy = &config.sections.user_url_policy;
    let validator = UrlValidator::new(
        SystemResolver,
        &policy.user_url_allowed_hosts,
        &Verification::Enabled,
    );
    let fetcher = Fetcher::new(
        client.clone(),
        UserUrlFetcher {
            validator,
            validation: policy.user_url_validation,
        },
        config.sections.image_url_download.max_download_size_mb,
        Arc::new(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |elapsed| elapsed.as_secs_f64())
        }),
    )?;
    Ok(fetcher)
}

fn option_defaults(
    values: &betterlite_config::sections::anthropic_option_primitives::OptionValues,
) -> OptionDefaults {
    OptionDefaults {
        max_tokens: values.max_tokens,
        stop_sequences: values.stop_sequences.clone(),
        temperature: values.temperature.and_then(serde_json::Number::from_f64),
        top_p: values.top_p.and_then(serde_json::Number::from_f64),
        top_k: values.top_k,
        metadata: values.metadata.clone(),
        system: values.system.clone(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error(transparent)]
    CostMap(#[from] ModelCostMapError),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    CatalogJson(#[from] serde_json::Error),
    #[error(transparent)]
    Patterns(#[from] CatalogLookupError),
    #[error(transparent)]
    Fetch(#[from] CacheError),
    #[error(transparent)]
    Redact(#[from] RedactionError),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("the bundled cl100k vocabulary is missing")]
    MissingVocabulary,
}
