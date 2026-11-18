use super::{Call, Scenario, services::Services};
use crate::anthropic_dispatch::base::Surface;
use crate::anthropic_dispatch::base::{Environment, Globals};
use crate::anthropic_dispatch::completion::CompletionCall;
use crate::anthropic_dispatch::completion::CompletionSettings;
use betterlite_anthropic::chat_call::CallStyle;
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::anthropic_completion::OptionValues;
use betterlite_config::types::ProxyTrust;
use betterlite_context::anthropic_catalog_lookup::{RawCatalog, candidates::CandidatePatterns};
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_core::cost::completion::lookup::NoHfConfig;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::decisions::Json;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::model_metadata_resolution::{
    hf_config::HfConfigClient, names::ModelCostMap,
};
use betterlite_prompts::factory::{PromptFactory, hub::Hub};
use betterlite_tokenizer::engine::{
    Engine,
    assets::bundled,
    tiktoken::{Encoding, TiktokenEngine},
};
use indexmap::IndexMap;
use secrecy::SecretString;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const COSTS: &[u8] = br#"{"claude-3-haiku-20240307":{"litellm_provider":"anthropic","supports_function_calling":true}}"#;

pub(super) struct Fixture {
    pub(super) client: ProxiedClient,
    pub(super) services: Services,
    pub(super) globals: Globals,
    pub(super) credentials: Environment,
    pub(super) custom_prompts: BTreeMap<String, Json>,
    pub(super) map: ModelCostMap,
    pub(super) catalog: ModelCatalog,
    pub(super) patterns: CandidatePatterns,
    pub(super) source: RawCatalog,
    pub(super) defaults: OptionDefaults,
    pub(super) legacy_defaults: OptionValues,
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
        let settings = AnthropicSettings::default();
        let services = Services::from((&client, &settings));
        Self {
            client,
            services,
            globals: Globals::default(),
            credentials: Environment::default(),
            custom_prompts: BTreeMap::new(),
            map: ModelCostMap::parse(COSTS).unwrap(),
            catalog: ModelCatalog::parse(COSTS, UnknownFields::Warn).unwrap(),
            patterns: CandidatePatterns::new().unwrap(),
            source: serde_json::from_slice(COSTS).unwrap(),
            legacy_defaults: OptionValues::default(),
            defaults: serde_json::from_str(r#"{"max_tokens":17}"#).unwrap(),
            settings,
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
impl Default for Fixture {
    fn default() -> Self {
        let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
        let client = ProxiedClient::outbound(&Arc::new(policy)).unwrap();
        Self::from(client)
    }
}
impl<'deps> From<(&'deps Fixture, ChatConfigTag)> for CompletionSettings<'deps> {
    fn from((fixture, selection): (&'deps Fixture, ChatConfigTag)) -> Self {
        Self {
            globals: &fixture.globals,
            environment: &fixture.credentials,
            custom_prompt_dict: &fixture.custom_prompts,
            disable_suffix: false,
            selection: Some(selection),
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
