use super::{Call, Scenario};
use crate::anthropic_dispatch::base::Surface;
use crate::anthropic_dispatch::completion::CompletionCall;
use crate::anthropic_dispatch::native::Runtime;
use betterlite_anthropic::anthropic_http_stream::call::{StreamLogger, StreamPostCall};
use betterlite_anthropic::chat_call::CallStyle;
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::stream_state::state::UuidV4;
use betterlite_context::anthropic_catalog_lookup::{RawCatalog, candidates::CandidatePatterns};
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_core::cost::completion::lookup::NoHfConfig;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::decisions::Json;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_metadata_resolution::{
    hf_config::HfConfigClient, names::ModelCostMap,
};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_prompts::factory::{PromptFactory, hub::Hub};
use betterlite_tokenizer::engine::{
    Engine,
    assets::bundled,
    tiktoken::{Encoding, TiktokenEngine},
};
use chrono::DateTime;
use indexmap::IndexMap;
use secrecy::SecretString;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const COSTS: &[u8] = br#"{"claude-3-haiku-20240307":{"litellm_provider":"anthropic","supports_function_calling":true}}"#;

pub(super) struct Fixture {
    pub(super) client: ProxiedClient,
    pub(super) map: ModelCostMap,
    pub(super) catalog: ModelCatalog,
    pub(super) patterns: CandidatePatterns,
    pub(super) source: RawCatalog,
    pub(super) defaults: OptionDefaults,
    pub(super) settings: AnthropicSettings,
    pub(super) matrix: BetaMatrix,
    pub(super) prompts: PromptFactory,
    pub(super) hf: HfConfigClient<NoHfConfig>,
    pub(super) generalizations: CatalogGeneralizations,
    pub(super) provider_catalog: Catalog,
    pub(super) names: BTreeSet<String>,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) new_id: fn() -> String,
    pub(super) engine: Engine,
}
impl From<ProxiedClient> for Fixture {
    fn from(client: ProxiedClient) -> Self {
        let encoding = Encoding::Cl100kBase;
        let engine = TiktokenEngine::from_bpe_file(encoding, &bundled(encoding).unwrap()).unwrap();
        Self {
            client,
            map: ModelCostMap::parse(COSTS).unwrap(),
            catalog: ModelCatalog::parse(COSTS, UnknownFields::Warn).unwrap(),
            patterns: CandidatePatterns::new().unwrap(),
            source: serde_json::from_slice(COSTS).unwrap(),
            defaults: serde_json::from_str(r#"{"max_tokens":17}"#).unwrap(),
            settings: AnthropicSettings::default(),
            matrix: serde_json::from_str(r#"{"anthropic":{}}"#).unwrap(),
            prompts: PromptFactory {
                cache: Mutex::new(BTreeMap::new()),
                hub: Hub::Unreachable,
            },
            hf: HfConfigClient {
                client: NoHfConfig,
                base_url: "https://metadata.invalid".to_owned(),
            },
            generalizations: CatalogGeneralizations::default(),
            provider_catalog: Catalog::new(),
            names: BTreeSet::new(),
            environment: BTreeMap::new(),
            new_id: || Uuid::new_v4().to_string(),
            engine: Engine::Tiktoken(Arc::new(engine)),
        }
    }
}
impl<'deps> From<&'deps Fixture> for ResolutionContext<'deps> {
    fn from(fixture: &'deps Fixture) -> Self {
        Self {
            catalog: &fixture.catalog,
            known_model_names: &fixture.names,
            custom_providers: &fixture.names,
            environment: &fixture.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}
struct Log;
impl StreamLogger for Log {
    fn post_call(&self, _call: StreamPostCall<'_>) {}
}
impl<'deps> From<&'deps Fixture> for Runtime<'deps, ProxiedClient> {
    fn from(fixture: &'deps Fixture) -> Self {
        Self {
            pooled: &fixture.client,
            settings: &fixture.settings,
            matrix: &fixture.matrix,
            stream_logger: &Log,
            ids: &UuidV4,
            now: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            max_event_bytes: 0x0001_0000,
            max_error_body_bytes: 0x0001_0000,
            disable_add_prefix_to_prompt: false,
            effort_used: false,
        }
    }
}
impl Fixture {
    pub(super) fn call(&self, scenario: &Scenario<'_>, base: String) -> Call {
        let mut optional_params = match scenario.surface {
            Surface::Messages => serde_json::from_str(r#"{"tools":[{"type":"function","function":{"name":"bad/name","parameters":{"type":"object","properties":{}}}}]}"#).unwrap(),
            Surface::Legacy => IndexMap::new(),
        };
        optional_params
            .extend(serde_json::from_str::<IndexMap<String, Json>>(scenario.options).unwrap());
        CompletionCall {
            model: match scenario.surface {
                Surface::Messages => "claude-3-haiku-20240307",
                Surface::Legacy => "claude-2",
            }
            .to_owned(),
            provider: "anthropic".to_owned(),
            messages: if scenario.key {
                serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).unwrap()
            } else {
                vec![Json::Bool(true)]
            },
            model_response: ResponseEnvelope::new("chatcmpl-native".to_owned()),
            stream: matches!(optional_params.get("stream"), Some(Json::Bool(true))),
            optional_params,
            litellm_params: IndexMap::new(),
            headers: BTreeMap::new(),
            api_key: scenario.key.then(|| SecretString::from("native-key")),
            api_base: Some(base),
            custom_prompt_dict: BTreeMap::new(),
            acompletion: scenario.style == CallStyle::Async,
            client: None,
            shared_session: None,
            encoding: self.engine.clone(),
            timeout: super::TIMEOUT,
        }
    }
}
