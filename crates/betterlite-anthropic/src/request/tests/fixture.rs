use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::decisions::Json;
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
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use indexmap::IndexMap;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::anthropic_capability_catalog::CatalogSources;
use crate::headers::AnthropicHeaders;
use crate::option_primitives::OptionDefaults;
use crate::params::DEFAULT_MAX_TOKENS;
use crate::request::error::RequestError;
use crate::request::{AnthropicRequest, PreparedRequest};

pub(crate) const MODELS: &str = r#"{
    "plain-model": {"litellm_provider": "anthropic"},
    "mid-model": {"litellm_provider": "anthropic", "supports_mid_conversation_system": true},
    "claude-sonnet-4-6": {"litellm_provider": "anthropic", "supports_mid_conversation_system": true},
    "always-on": {"litellm_provider": "anthropic", "thinking_always_on": true},
    "effort-model": {"litellm_provider": "anthropic", "supports_output_config": true, "supports_max_reasoning_effort": true, "supports_xhigh_reasoning_effort": true},
    "fast-model": {"litellm_provider": "anthropic", "supports_speed": true},
    "no-sampling": {"litellm_provider": "anthropic", "supports_sampling_params": false}
}"#;

const ID: &str = "generated-id";

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split,
};

// The real splitter lives in betterlite-bedrock, which a provider leaf cannot depend on.
fn mantle_region_split(model: &str) -> (Option<&str>, &str) {
    match model.split_once('/') {
        Some((prefix, rest)) if prefix == "us-east-1" => (Some(prefix), rest),
        _ => (None, model),
    }
}

pub(crate) struct NoHfTransport;

#[derive(Debug, thiserror::Error)]
#[error("the metadata lookup must not fetch a Hugging Face config")]
pub(crate) struct NoHfError;

impl ModelInfoClient for NoHfTransport {
    type Error = NoHfError;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, NoHfError>> + Send {
        ready(Err(NoHfError))
    }
}

// The current no-download seam: the fetch is attempted and refused with the pinned 400 error
#[derive(Default)]
pub(crate) struct RefusingFetcher {
    calls: AtomicUsize,
}

impl RefusingFetcher {
    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

fn refused(url: &str) -> ImageFetchError {
    ImageFetchError::Disabled {
        url: url.to_owned(),
    }
}

impl RemoteMediaFetcher for RefusingFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ready(Err(refused(url)))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let url = urls.first().map_or("", String::as_str);
        ready(Err(refused(url)))
    }
}

// A gateway that answers the batch: the inline stage must spend it before translation, so a single fetch still refuses
pub(crate) const INLINED_DATA_URL: &str = "data:image/png;base64,QUJD";

pub(crate) struct InliningFetcher;

impl RemoteMediaFetcher for InliningFetcher {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(refused(url)))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(Ok(vec![INLINED_DATA_URL.to_owned(); urls.len()]))
    }
}

pub(crate) struct NoFileBlocks;

#[derive(Debug, thiserror::Error)]
#[error("the case supplies no file blocks")]
pub(crate) struct NoFileError;

impl FileBlocks for NoFileBlocks {
    type Failure = NoFileError;

    fn file_block(&self, _element: &Object) -> Result<Object, NoFileError> {
        Err(NoFileError)
    }
}

#[derive(Clone, Default)]
pub(crate) struct Warnings(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Warnings {
    pub(crate) fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(crate) fn capture_warnings() -> (DefaultGuard, Warnings) {
    let warnings = Warnings::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    (set_default(subscriber), warnings)
}

pub(crate) struct Fixture {
    map: ModelCostMap,
    catalog: ModelCatalog,
    generalizations: CatalogGeneralizations,
    provider_catalog: Catalog,
    hf: HfConfigClient<NoHfTransport>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    patterns: CandidatePatterns,
    runtime: RawCatalog,
    packaged: RawCatalog,
}

impl Fixture {
    fn new(model_cost: &str) -> Self {
        Self {
            map: ModelCostMap::parse(model_cost.as_bytes()).expect("model cost map"),
            catalog: ModelCatalog::parse(b"{}", UnknownFields::Warn).expect("empty catalog"),
            generalizations: CatalogGeneralizations::default(),
            provider_catalog: Catalog::new(),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            patterns: CandidatePatterns::new().expect("candidate patterns"),
            runtime: serde_json::from_str(model_cost).expect("runtime catalog"),
            packaged: serde_json::from_str("{}").expect("packaged catalog"),
        }
    }

    fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime,
            packaged: &self.packaged,
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    fn environment<'fixture>(
        &'fixture self,
        resolution: &'fixture ResolutionContext<'fixture>,
        keys: &'fixture mut ModelCostKeys,
        dynamic: &'fixture NoDynamicMetadata,
    ) -> CapabilityEnvironment<'fixture, NoDynamicMetadata, NoHfTransport> {
        CapabilityEnvironment {
            metadata: MetadataEnvironment {
                model_cost: &self.map.entries,
                vertex: &self.map.vertex,
                generalizations: &self.generalizations,
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

pub(crate) struct Session<'fixture> {
    fixture: &'fixture Fixture,
    resolution: ResolutionContext<'fixture>,
    keys: ModelCostKeys,
    dynamic: NoDynamicMetadata,
}

impl<'fixture> Session<'fixture> {
    fn new(fixture: &'fixture Fixture) -> Self {
        Self {
            fixture,
            resolution: fixture.resolution(),
            keys: ModelCostKeys::default(),
            dynamic: NoDynamicMetadata,
        }
    }

    fn environment(&mut self) -> CapabilityEnvironment<'_, NoDynamicMetadata, NoHfTransport> {
        self.fixture
            .environment(&self.resolution, &mut self.keys, &self.dynamic)
    }
}

pub(crate) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("case JSON")
}

pub(crate) fn fields(source: &str) -> IndexMap<String, Json> {
    match json(source) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            IndexMap::new()
        }
    }
}

pub(crate) fn messages(source: &str) -> Vec<Json> {
    match json(source) {
        Json::List(items) => items,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Vec::new()
        }
    }
}

pub(crate) fn string_map(source: &str) -> IndexMap<String, String> {
    let mut values = IndexMap::new();
    for (key, value) in fields(source) {
        if let Json::Text(text) = value {
            values.insert(key, text);
        }
    }
    values
}

pub(crate) struct Scenario {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) messages: Vec<Json>,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) litellm_params: IndexMap<String, Json>,
    pub(crate) headers: IndexMap<String, String>,
    pub(crate) defaults: OptionDefaults,
    pub(crate) default_chat_max_tokens: i64,
    pub(crate) strips_billing_metadata: bool,
    pub(crate) modify_params: bool,
    pub(crate) global_drop_params: bool,
}

impl Scenario {
    pub(crate) fn new(model: &str, source: &str) -> Self {
        Self {
            model: model.to_owned(),
            custom_llm_provider: None,
            messages: messages(source),
            optional_params: IndexMap::new(),
            litellm_params: IndexMap::new(),
            headers: IndexMap::new(),
            defaults: OptionDefaults::default(),
            default_chat_max_tokens: DEFAULT_MAX_TOKENS,
            strips_billing_metadata: false,
            modify_params: false,
            global_drop_params: false,
        }
    }

    pub(crate) fn optional(mut self, source: &str) -> Self {
        self.optional_params = fields(source);
        self
    }

    pub(crate) fn litellm(mut self, source: &str) -> Self {
        self.litellm_params = fields(source);
        self
    }

    pub(crate) fn headers(mut self, source: &str) -> Self {
        self.headers = string_map(source);
        self
    }

    pub(crate) fn defaults(mut self, defaults: OptionDefaults) -> Self {
        self.defaults = defaults;
        self
    }

    pub(crate) fn provider(mut self, provider: &str) -> Self {
        self.custom_llm_provider = Some(provider.to_owned());
        self
    }

    pub(crate) fn modify(mut self) -> Self {
        self.modify_params = true;
        self
    }

    pub(crate) fn strip_billing(mut self) -> Self {
        self.strips_billing_metadata = true;
        self
    }

    pub(crate) fn global_drop(mut self) -> Self {
        self.global_drop_params = true;
        self
    }

    pub(crate) fn run(&self) -> Outcome {
        let fetcher = RefusingFetcher::default();
        let mut outcome = self.run_with(&fetcher);
        outcome.fetch_calls = fetcher.calls();
        outcome
    }

    pub(crate) fn run_with<F>(&self, fetcher: &F) -> Outcome
    where
        F: RemoteMediaFetcher + Sync,
    {
        let fixture = Fixture::new(MODELS);
        let mut session = Session::new(&fixture);
        let (guard, warnings) = capture_warnings();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let catalog = ModelCatalog::parse(b"{}", UnknownFields::Warn).expect("empty catalog");
        let result = runtime.block_on(self.prepare(&mut session, &fixture, fetcher, &catalog));
        drop(guard);
        Outcome::new(result, warnings.messages(), 0)
    }

    async fn prepare<F>(
        &self,
        session: &mut Session<'_>,
        fixture: &Fixture,
        fetcher: &F,
        catalog: &ModelCatalog,
    ) -> Result<PreparedRequest, RequestError<NoFileError>>
    where
        F: RemoteMediaFetcher + Sync,
    {
        let mut environment = session.environment();
        let catalogs = fixture.catalogs();
        let request = AnthropicRequest {
            model: &self.model,
            custom_llm_provider: self.custom_llm_provider.as_deref(),
            messages: self.messages.clone(),
            optional_params: self.optional_params.clone(),
            litellm_params: self.litellm_params.clone(),
            headers: AnthropicHeaders {
                values: self.headers.clone(),
            },
            defaults: &self.defaults,
            catalog,
            default_chat_max_tokens: self.default_chat_max_tokens,
            strips_billing_metadata: self.strips_billing_metadata,
            modify_params: self.modify_params,
            global_drop_params: self.global_drop_params,
        };
        let blocks = NoFileBlocks;
        let new_id = || ID.to_owned();
        let prompts = PromptDeps {
            fetcher,
            blocks: &blocks,
            new_id: &new_id,
        };
        request.prepare(&mut environment, &catalogs, &prompts).await
    }
}

pub(crate) struct Outcome {
    pub(crate) body: IndexMap<String, Json>,
    pub(crate) headers: IndexMap<String, String>,
    pub(crate) reverse: BTreeMap<String, String>,
    pub(crate) failure: Option<(Option<u16>, String)>,
    pub(crate) warnings: Vec<String>,
    pub(crate) fetch_calls: usize,
}

impl Outcome {
    fn new(
        result: Result<PreparedRequest, RequestError<NoFileError>>,
        warnings: Vec<String>,
        fetch_calls: usize,
    ) -> Self {
        match result {
            Ok(prepared) => Self {
                body: prepared.body,
                headers: prepared.headers.values,
                reverse: prepared.tool_name_reverse,
                failure: None,
                warnings,
                fetch_calls,
            },
            Err(error) => Self {
                body: IndexMap::new(),
                headers: IndexMap::new(),
                reverse: BTreeMap::new(),
                failure: Some((error.status_code(), error.to_string())),
                warnings,
                fetch_calls,
            },
        }
    }

    pub(crate) fn ok(self) -> Self {
        assert!(self.failure.is_none(), "request failed: {:?}", self.failure);
        self
    }

    pub(crate) fn failed(mut self) -> (Option<u16>, String) {
        self.failure.take().expect("the request raises")
    }

    pub(crate) fn get(&self, key: &str) -> Option<&Json> {
        self.body.get(key)
    }

    pub(crate) fn beta(&self) -> Option<&str> {
        self.headers.get("anthropic-beta").map(String::as_str)
    }
}

// Both sides read as the crate's own JSON tree, whose object equality ignores key order
pub(crate) fn expect_field(outcome: &Outcome, key: &str, expected: &str) {
    let actual = outcome
        .get(key)
        .unwrap_or_else(|| panic!("the body has no {key}"));
    assert_eq!(*actual, json(expected), "{key}");
}

pub(crate) fn expect_absent(outcome: &Outcome, key: &str) {
    assert!(
        outcome.get(key).is_none(),
        "the body still carries {key}: {:?}",
        outcome.get(key)
    );
}
