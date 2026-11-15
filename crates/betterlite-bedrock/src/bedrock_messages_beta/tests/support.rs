use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::feature_flags::FeatureParams;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
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

use crate::bedrock_messages_beta::beta::{BetaHeadersError, BetaHeadersRequest, beta_headers};
use crate::bedrock_messages_beta::tool_search::adjust_tool_search_beta;
use crate::event_frames::beta::BetaHeader;
use crate::mantle_inputs::region::split_region_prefix;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

pub(super) const MODEL_COST: &str = r#"{
  "bedrock/claude-sonnet-4-5": {"litellm_provider": "bedrock", "supports_tool_search": false},
  "bedrock/fixture-tool-search": {"litellm_provider": "bedrock", "supports_tool_search": true},
  "bedrock/fixture-no-flag": {"litellm_provider": "bedrock", "mode": "chat"}
}"#;

// One provider entry per token these cases send, copied from the bundled beta config.
pub(super) const MATRIX: &str = r#"{
  "anthropic": {
    "advanced-tool-use-2025-11-20": "advanced-tool-use-2025-11-20",
    "code-execution-2025-05-22": "code-execution-2025-05-22",
    "computer-use-2025-01-24": "computer-use-2025-01-24",
    "context-management-2025-06-27": "context-management-2025-06-27",
    "compact-2026-01-12": "compact-2026-01-12",
    "dangerous-tool-use-2026-09-03": "dangerous-tool-use-2026-09-03",
    "effort-2025-11-24": "effort-2025-11-24",
    "files-api-2025-04-14": "files-api-2025-04-14",
    "fine-grained-tool-streaming-2025-05-14": "fine-grained-tool-streaming-2025-05-14",
    "interleaved-thinking-2025-05-14": "interleaved-thinking-2025-05-14",
    "mcp-client-2025-04-04": "mcp-client-2025-04-04",
    "mid-conversation-output-config-2026-07-01": "mid-conversation-output-config-2026-07-01",
    "mid-conversation-tool-changes-2026-07-01": "mid-conversation-tool-changes-2026-07-01",
    "thinking-display-updates-2026-08-18": "thinking-display-updates-2026-08-18"
  },
  "bedrock": {
    "advanced-tool-use-2025-11-20": "tool-search-tool-2025-10-19",
    "compact-2026-01-12": "compact-2026-01-12",
    "context-management-2025-06-27": "context-management-2025-06-27",
    "dangerous-tool-use-2026-09-03": "dangerous-tool-use-2026-09-03",
    "fine-grained-tool-streaming-2025-05-14": "fine-grained-tool-streaming-2025-05-14",
    "tool-search-tool-2025-10-19": "tool-search-tool-2025-10-19",
    "tool-examples-2025-10-29": null
  },
  "bedrock_mantle": {
    "advanced-tool-use-2025-11-20": "tool-search-tool-2025-10-19",
    "compact-2026-01-12": "compact-2026-01-12",
    "context-management-2025-06-27": "context-management-2025-06-27",
    "dangerous-tool-use-2026-09-03": "dangerous-tool-use-2026-09-03",
    "effort-2025-11-24": "effort-2025-11-24",
    "fine-grained-tool-streaming-2025-05-14": "fine-grained-tool-streaming-2025-05-14",
    "interleaved-thinking-2025-05-14": "interleaved-thinking-2025-05-14",
    "tool-examples-2025-10-29": "tool-examples-2025-10-29",
    "tool-search-tool-2025-10-19": "tool-search-tool-2025-10-19"
  }
}"#;

pub(super) fn js(text: &str) -> Json {
    serde_json::from_str(text).expect("fixture JSON parses")
}

pub(super) fn obj(text: &str) -> Object {
    let Json::Object(object) = js(text) else {
        panic!("fixture JSON is an object");
    };
    object
}

pub(super) fn list(text: &str) -> Vec<Json> {
    let Json::List(items) = js(text) else {
        panic!("fixture JSON is a list");
    };
    items
}

pub(super) fn headers(text: &str) -> BTreeMap<String, BetaHeader> {
    serde_json::from_str(text).expect("fixture headers parse")
}

pub(super) fn matrix() -> BetaMatrix {
    serde_json::from_str(MATRIX).expect("the fixture matrix parses")
}

pub(super) fn params(text: &str) -> FeatureParams {
    serde_json::from_str(text).expect("fixture feature params parse")
}

pub(super) struct Call {
    pub(super) model: String,
    pub(super) messages: Vec<Json>,
    pub(super) tools: Vec<Json>,
    pub(super) mcp_servers: Vec<Json>,
    pub(super) safeguards: Option<Json>,
    pub(super) params: FeatureParams,
    pub(super) header_values: BTreeMap<String, BetaHeader>,
    pub(super) outgoing: Object,
    pub(super) injected: bool,
    pub(super) subclass_provider: Option<String>,
}

impl Call {
    pub(super) fn new(model: &str) -> Self {
        Self {
            model: model.to_owned(),
            messages: Vec::new(),
            tools: Vec::new(),
            mcp_servers: Vec::new(),
            safeguards: None,
            params: FeatureParams::default(),
            header_values: BTreeMap::new(),
            outgoing: Object::new(),
            injected: false,
            subclass_provider: None,
        }
    }

    pub(super) fn request(&mut self) -> BetaHeadersRequest<'_> {
        BetaHeadersRequest {
            model: &self.model,
            messages: &self.messages,
            tools: &self.tools,
            mcp_servers: &self.mcp_servers,
            safeguards: self.safeguards.as_ref(),
            params: &self.params,
            headers: &self.header_values,
            outgoing: &mut self.outgoing,
            clear_thinking_injected: self.injected,
            subclass_provider: self.subclass_provider.as_deref(),
        }
    }
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the test runtime builds")
}

#[derive(Debug, thiserror::Error)]
#[error("the metadata lookup must not fetch a Hugging Face config")]
pub(super) struct NoHfError;

pub(super) struct NoHfTransport;

impl ModelInfoClient for NoHfTransport {
    type Error = NoHfError;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, NoHfError>> + Send {
        ready(Err(NoHfError))
    }
}

pub(super) struct Fixture {
    entries: ModelCostMap,
    typed: ModelCatalog,
    packaged: Arc<ModelCatalog>,
    provider_catalog: Catalog,
    hf: HfConfigClient<NoHfTransport>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    patterns: CandidatePatterns,
    runtime: RawCatalog,
    unlisted: RawCatalog,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let entries =
            ModelCostMap::parse(MODEL_COST.as_bytes()).expect("the model cost map parses");
        let patterns = CandidatePatterns::new().expect("the candidate patterns compile");
        let runtime = serde_json::from_str(MODEL_COST).expect("the raw catalog parses");
        let typed = ModelCatalog::parse(b"{}", UnknownFields::Warn).expect("an empty catalog");
        let packaged = BundledCatalog::default().get();
        let client = NoHfTransport;
        let base_url = HF_CONFIG_BASE_URL.to_owned();
        Self {
            entries,
            typed,
            packaged,
            provider_catalog: Catalog::new(),
            hf: HfConfigClient { client, base_url },
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            patterns,
            runtime,
            unlisted: RawCatalog::new(),
        }
    }

    pub(super) const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime,
            packaged: &self.unlisted,
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.typed,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    fn capability_environment<'fixture>(
        &'fixture self,
        resolution: &'fixture ResolutionContext<'fixture>,
        keys: &'fixture mut ModelCostKeys,
        dynamic: &'fixture NoDynamicMetadata,
    ) -> CapabilityEnvironment<'fixture, NoDynamicMetadata, NoHfTransport> {
        CapabilityEnvironment {
            metadata: MetadataEnvironment {
                model_cost: &self.entries.entries,
                vertex: &self.entries.vertex,
                generalizations: &self.packaged.generalizations,
                resolution,
                normalizers: NORMALIZERS,
            },
            keys,
            dynamic,
            hf: &self.hf,
            provider_catalog: &self.provider_catalog,
            provider_generation: 0,
        }
    }
}

pub(super) struct Session<'fixture> {
    fixture: &'fixture Fixture,
    resolution: ResolutionContext<'fixture>,
    keys: ModelCostKeys,
    runtime: Runtime,
}

impl<'fixture> Session<'fixture> {
    pub(super) fn new(fixture: &'fixture Fixture) -> Self {
        let resolution = fixture.resolution();
        let runtime = runtime();
        Self {
            fixture,
            resolution,
            keys: ModelCostKeys::default(),
            runtime,
        }
    }

    pub(super) fn betas(
        &mut self,
        request: BetaHeadersRequest<'_>,
    ) -> Result<Vec<String>, BetaHeadersError> {
        let catalogs = self.fixture.catalogs();
        let mut environment = self.fixture.capability_environment(
            &self.resolution,
            &mut self.keys,
            &NoDynamicMetadata,
        );
        self.runtime.block_on(beta_headers(
            request,
            &mut environment,
            &catalogs,
            &matrix(),
        ))
    }

    pub(super) fn adjust(&mut self, model: &str, tools: &[Json], betas: &mut BTreeSet<String>) {
        let catalogs = self.fixture.catalogs();
        let mut environment = self.fixture.capability_environment(
            &self.resolution,
            &mut self.keys,
            &NoDynamicMetadata,
        );
        self.runtime.block_on(adjust_tool_search_beta(
            model,
            super::super::tool_search::beta_mode(tools),
            betas,
            &mut environment,
            &catalogs,
        ));
    }
}

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<(Level, String)>>>);

impl Logs {
    pub(super) fn warnings(&self) -> Vec<String> {
        self.messages(Level::WARN)
    }

    pub(super) fn debugs(&self) -> Vec<String> {
        self.messages(Level::DEBUG)
    }

    fn messages(&self, level: Level) -> Vec<String> {
        let records = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let mut messages = Vec::new();
        for (recorded, message) in records.iter() {
            if *recorded == level {
                messages.push(message.clone());
            }
        }
        drop(records);
        messages
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
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

pub(super) fn capture_logs() -> (DefaultGuard, Logs) {
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    (set_default(subscriber), logs)
}
