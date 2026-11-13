use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::MetadataEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_models::model_metadata_resolution::names::{ModelCostMap, NameNormalizers};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_prompts::arguments::json::{Json, Object};
use tokio::runtime::{Builder, Runtime};
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::api::messages_native_transform::capabilities::AnthropicCapabilities;
use crate::api::messages_native_transform::error::TransformError;
use crate::api::messages_native_transform::request::{TransformRequest, transform_native_request};

pub(crate) const RECORDS: &str = r#"{
    "claude-sonnet-4-5": {"litellm_provider": "anthropic", "supports_reasoning": true, "supports_adaptive_thinking": false, "supports_output_config": false, "supports_legacy_thinking": false, "supports_xhigh_reasoning_effort": false, "supports_max_reasoning_effort": false, "thinking_always_on": false},
    "claude-opus-4-5": {"litellm_provider": "anthropic", "supports_reasoning": true, "supports_adaptive_thinking": false, "supports_output_config": true, "supports_legacy_thinking": false, "supports_xhigh_reasoning_effort": false, "supports_max_reasoning_effort": false, "thinking_always_on": false},
    "claude-opus-4-6": {"litellm_provider": "anthropic", "supports_reasoning": true, "supports_adaptive_thinking": true, "supports_output_config": true, "supports_legacy_thinking": true, "supports_xhigh_reasoning_effort": false, "supports_max_reasoning_effort": true, "thinking_always_on": false}
}"#;

fn region_split(model: &str) -> (Option<&str>, &str) {
    (None, model)
}

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: region_split,
};

pub(crate) struct Offline;

#[derive(Debug, thiserror::Error)]
#[error("the fixture never fetches a Hugging Face config")]
pub(crate) struct OfflineError;

impl ModelInfoClient for Offline {
    type Error = OfflineError;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, OfflineError>> + Send {
        ready(Err(OfflineError))
    }
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<(Level, String)>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Captured {
    pub(crate) fn warnings(&self) -> Vec<String> {
        self.events()
            .into_iter()
            .filter(|(level, _message)| *level == Level::WARN)
            .map(|(_level, message)| message)
            .collect()
    }

    pub(crate) fn logged_transformation(&self) -> bool {
        self.events().iter().any(|(level, message)| {
            *level == Level::DEBUG && message.contains("TRANSFORMATION DEBUG")
        })
    }

    fn events(&self) -> Vec<(Level, String)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        if level != Level::WARN && level != Level::DEBUG {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((level, message.0));
    }
}

pub(crate) fn capture() -> (DefaultGuard, Captured) {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::registry().with(captured.clone());
    (set_default(subscriber), captured)
}

pub(crate) struct World {
    map: ModelCostMap,
    typed: ModelCatalog,
    generalizations: CatalogGeneralizations,
    provider_catalog: Catalog,
    hf: HfConfigClient<Offline>,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    patterns: CandidatePatterns,
    runtime: RawCatalog,
    packaged: RawCatalog,
    budgets: Settings,
}

impl World {
    pub(crate) fn load(source: &str) -> Self {
        Self {
            map: ModelCostMap::parse(source.as_bytes()).expect("pinned capability records"),
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn).expect("empty catalog"),
            generalizations: CatalogGeneralizations::default(),
            provider_catalog: Catalog::new(),
            hf: HfConfigClient {
                client: Offline,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
            patterns: CandidatePatterns::new().expect("candidate patterns"),
            runtime: serde_json::from_str(source).expect("runtime catalog"),
            packaged: RawCatalog::default(),
            budgets: Settings::default(),
        }
    }

    pub(crate) fn transform(
        &self,
        request: TransformRequest<'_>,
    ) -> Result<Object, TransformError> {
        let resolution = ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        let mut keys = ModelCostKeys::default();
        let dynamic = NoDynamicMetadata;
        let mut environment = CapabilityEnvironment {
            metadata: MetadataEnvironment {
                model_cost: &self.map.entries,
                vertex: &self.map.vertex,
                generalizations: &self.generalizations,
                resolution: &resolution,
                normalizers: NORMALIZERS,
            },
            keys: &mut keys,
            dynamic: &dynamic,
            hf: &self.hf,
            provider_catalog: &self.provider_catalog,
            provider_generation: 0,
        };
        let catalogs = CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime,
            packaged: &self.packaged,
        };
        let mut capabilities = AnthropicCapabilities {
            environment: &mut environment,
            catalogs: &catalogs,
            budgets: &self.budgets,
            llm_provider: "anthropic",
        };
        runtime().block_on(transform_native_request(&mut capabilities, request))
    }
}

pub(crate) fn transform(
    fixture: &World,
    name: &str,
    history: &str,
    options: &str,
    strips_billing_metadata: bool,
) -> Result<Object, TransformError> {
    fixture.transform(TransformRequest {
        model: name,
        messages: parsed_messages(history),
        optional_params: object(options),
        custom_llm_provider: "anthropic",
        strips_billing_metadata,
    })
}

fn runtime() -> Runtime {
    Builder::new_current_thread().build().expect("test runtime")
}

fn parsed_messages(source: &str) -> Vec<Json> {
    serde_json::from_str(source).expect("fixture messages")
}

pub(crate) fn body_of(result: Result<Object, TransformError>) -> Json {
    Json::Object(result.expect("the request transform"))
}

pub(crate) fn field<'body>(body: &'body Json, key: &str) -> Option<&'body Json> {
    match body {
        Json::Object(fields) => fields.get(key),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(crate) fn failure(result: Result<Object, TransformError>) -> TransformError {
    result.expect_err("a rejected request")
}

pub(crate) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("fixture JSON")
}

pub(crate) fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("fixture object")
}
