use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use aws_smithy_eventstream::frame::write_message_to;
use aws_smithy_types::event_stream::{Header, HeaderValue, Message};
use aws_smithy_types::str_bytes::StrBytes;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_core::api::anthropic_beta_config::matrix::BetaMatrix;
use betterlite_core::api::messages_sse_events::stream::MessagesProviderStream;
use betterlite_http::error::HttpError;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::full_model_metadata::record::ModelInfo;
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
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::super::agentcore_stream::transport::Chunks;
use super::super::aws_settings::sources::Credentials;
use super::super::bedrock_model_capabilities::ModelInfoLookup;
use super::super::credentials::error::CredentialError;
use super::super::event_frames::beta::BetaHeader;
use super::super::invoke_primitives::config::Params as InvokeParams;
use super::super::mantle_inputs::region::split_region_prefix;
use super::super::messages::stream::{
    MessagesStreamError, MessagesStreamRequest, messages_sse_stream,
};
use super::super::messages::transform::drop_unsupported_output_config;
use super::super::messages::transform::{
    MessagesRequest, MessagesTransformError, TransformSources, transform_anthropic_messages_request,
};
use super::super::messages::transport::{MessagesSignRequest, sign_messages_request};
use super::super::model_routes::ParamMap;
use super::super::signing::error::SigningError;
use super::super::signing::request::AwsSignedRequest;
use super::super::signing::target::RegionInputs;

const BUNDLED_BETA_CONFIG: &str = include_str!(
    "../../../../../crates/betterlite-core/src/api/anthropic_beta_config/anthropic_beta_headers_config.json"
);

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

// The same cost map the oracle cases send, so both sides resolve the same capabilities.
pub(super) const MODEL_COST: &str = r#"{
  "bedrock/fixture-plain": {"litellm_provider": "bedrock", "mode": "chat"},
  "fixture-plain": {"litellm_provider": "bedrock", "mode": "chat"},
  "bedrock/fixture-tool-search": {"litellm_provider": "bedrock", "supports_tool_search": true},
  "fixture-tool-search": {"litellm_provider": "bedrock", "supports_tool_search": true},
  "bedrock/fixture-adaptive": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true},
  "fixture-adaptive": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true},
  "bedrock/fixture-effort-tier": {"litellm_provider": "bedrock", "supports_xhigh_reasoning_effort": true, "bedrock_output_config_effort_ceiling": "high"},
  "fixture-effort-tier": {"litellm_provider": "bedrock", "supports_xhigh_reasoning_effort": true, "bedrock_output_config_effort_ceiling": "high"}
}"#;

pub(super) const EFFORT_TIER_INFO: &str =
    r#"{"fixture-effort-tier": {"bedrock_output_config_effort_ceiling": "high"}}"#;

pub(super) fn js(text: &str) -> Json {
    serde_json::from_str(text).expect("fixture JSON parses")
}

pub(super) fn contract(text: &str) -> IndexMap<String, ContractJson> {
    serde_json::from_str(text).expect("fixture contract JSON parses")
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

pub(super) fn body_json(body: &Object) -> Json {
    Json::Object(body.clone())
}

pub(super) struct Call {
    pub(super) model: String,
    pub(super) messages: Vec<Json>,
    pub(super) optional_params: Object,
    pub(super) header_values: BTreeMap<String, BetaHeader>,
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

#[derive(Debug, thiserror::Error)]
#[error("the model is not mapped")]
pub(super) struct Unmapped;

pub(super) struct Answers(BTreeMap<String, IndexMap<String, ContractJson>>);

impl ModelInfoLookup for Answers {
    type Error = Unmapped;

    fn bedrock_model_info(
        &self,
        model: &str,
    ) -> impl Future<Output = Result<ModelInfo, Unmapped>> + Send {
        let answer = self.0.get(model).cloned();
        ready(answer.map(|fields| ModelInfo { fields }).ok_or(Unmapped))
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
    model_cost: ParamMap,
    model_info: BTreeMap<String, IndexMap<String, ContractJson>>,
    budgets: Settings,
    matrix: BetaMatrix,
}

impl Fixture {
    pub(super) fn new(model_info: &str) -> Self {
        let bytes = MODEL_COST.as_bytes();
        let entries = ModelCostMap::parse(bytes).expect("the model cost map parses");
        let runtime: RawCatalog = serde_json::from_slice(bytes).expect("the raw catalog parses");
        let typed = ModelCatalog::parse(b"{}", UnknownFields::Warn).expect("an empty catalog");
        let packaged = BundledCatalog::default().get();
        Self {
            entries,
            typed,
            packaged,
            provider_catalog: Catalog::new(),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            patterns: CandidatePatterns::new().expect("the candidate patterns compile"),
            runtime,
            unlisted: RawCatalog::new(),
            model_cost: serde_json::from_str(MODEL_COST).expect("the model cost params parse"),
            model_info: serde_json::from_str(model_info).expect("the model info parses"),
            budgets: Settings::default(),
            matrix: serde_json::from_str(BUNDLED_BETA_CONFIG).expect("the bundled matrix parses"),
        }
    }

    const fn catalogs(&self) -> CatalogSources<'_> {
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
    dynamic: NoDynamicMetadata,
    runtime: Runtime,
    lookup: Answers,
}

impl<'fixture> Session<'fixture> {
    pub(super) fn new(fixture: &'fixture Fixture) -> Self {
        Self {
            fixture,
            resolution: fixture.resolution(),
            keys: ModelCostKeys::default(),
            dynamic: NoDynamicMetadata,
            runtime: Builder::new_current_thread()
                .build()
                .expect("the test runtime builds"),
            lookup: Answers(fixture.model_info.clone()),
        }
    }

    pub(super) fn transform(
        &mut self,
        call: &Call,
        drop_params: bool,
    ) -> Result<Object, MessagesTransformError> {
        let catalogs = self.fixture.catalogs();
        let mut environment =
            self.fixture
                .capability_environment(&self.resolution, &mut self.keys, &self.dynamic);
        let request = MessagesRequest {
            model: &call.model,
            messages: call.messages.clone(),
            optional_params: call.optional_params.clone(),
            headers: &call.header_values,
            model_cost: &self.fixture.model_cost,
            drop_params,
        };
        let mut sources = TransformSources {
            environment: &mut environment,
            catalogs: &catalogs,
            budgets: &self.fixture.budgets,
            matrix: &self.fixture.matrix,
            lookup: &self.lookup,
            packaged: self.fixture.packaged.as_ref(),
        };
        self.runtime
            .block_on(transform_anthropic_messages_request(&request, &mut sources))
    }

    pub(super) fn drop_output_config(&mut self, body: &mut Object, model: &str, drop_params: bool) {
        let catalogs = self.fixture.catalogs();
        let mut environment =
            self.fixture
                .capability_environment(&self.resolution, &mut self.keys, &self.dynamic);
        let header_values = BTreeMap::new();
        let request = MessagesRequest {
            model,
            messages: Vec::new(),
            optional_params: Object::new(),
            headers: &header_values,
            model_cost: &self.fixture.model_cost,
            drop_params,
        };
        self.runtime.block_on(drop_unsupported_output_config(
            body,
            &request,
            &mut environment,
            &catalogs,
        ));
    }
}

pub(super) fn call(model: &str, messages: &str, params: &str, header_values: &str) -> Call {
    Call {
        model: model.to_owned(),
        messages: list(messages),
        optional_params: obj(params),
        header_values: headers(header_values),
    }
}

pub(super) fn declared(call: &Call, drop_params: bool) -> Result<Object, MessagesTransformError> {
    let fixture = Fixture::new(EFFORT_TIER_INFO);
    let mut session = Session::new(&fixture);
    session.transform(call, drop_params)
}

pub(super) fn kept_warnings(logs: &Logs) -> Vec<String> {
    logs.warnings()
        .into_iter()
        .filter(|message| {
            message.starts_with("Dropping ") || message.starts_with("Bedrock Invoke: ")
        })
        .collect()
}

pub(super) fn frame(entries: &[(&str, HeaderValue)], payload: &[u8]) -> Vec<u8> {
    let headers = entries
        .iter()
        .map(|(name, value)| Header::new((*name).to_owned(), value.clone()))
        .collect();
    let message = Message::new_from_parts(headers, payload.to_vec());
    let mut out = Vec::new();
    write_message_to(&message, &mut out).expect("the test frame writes");
    out
}

pub(super) fn text(value: &str) -> HeaderValue {
    HeaderValue::String(StrBytes::from(value.to_owned()))
}

pub(super) fn chunk_frame(body: &str) -> Vec<u8> {
    let payload = format!("{{\"bytes\":\"{}\"}}", STANDARD.encode(body));
    frame(
        &[
            (":event-type", text("chunk")),
            (":content-type", text("application/json")),
            (":message-type", text("event")),
        ],
        payload.as_bytes(),
    )
}

pub(super) fn error_frame(exception_type: &str, payload: &[u8]) -> Vec<u8> {
    frame(
        &[
            (":exception-type", text(exception_type)),
            (":content-type", text("application/json")),
            (":message-type", text("exception")),
        ],
        payload,
    )
}

pub(super) fn chunks(pieces: Vec<Vec<u8>>) -> Chunks {
    futures_util::stream::iter(pieces.into_iter().map(Ok::<Vec<u8>, HttpError>)).boxed()
}

pub(super) fn stream_all(
    response_headers: Option<BTreeMap<String, String>>,
    pieces: Vec<Vec<u8>>,
) -> (Vec<Vec<u8>>, Option<MessagesStreamError>) {
    let redactor = Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"));
    let mut stream = messages_sse_stream(
        MessagesStreamRequest {
            response_headers,
            redactor,
        },
        chunks(pieces),
    );
    let runtime = Builder::new_current_thread()
        .build()
        .expect("the test runtime builds");
    runtime.block_on(async {
        let mut emitted = Vec::new();
        let mut failure = None;
        loop {
            match stream.next_chunk().await {
                Ok(Some(piece)) => emitted.push(piece),
                Ok(None) => break,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        (emitted, failure)
    })
}

pub(super) fn header<'pairs>(pairs: &'pairs [(String, String)], name: &str) -> Option<&'pairs str> {
    pairs
        .iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, value)| value.as_str())
}

pub(super) const FIXTURE_NOW: u64 = 1_700_000_000;

pub(super) struct Keys {
    pub(super) access_key_id: &'static str,
    pub(super) secret_access_key: &'static str,
    pub(super) session_token: Option<&'static str>,
}

pub(super) struct Signing<'request> {
    pub(super) model: &'request str,
    pub(super) headers: &'request IndexMap<String, String>,
    pub(super) request_data: &'request IndexMap<String, ContractJson>,
    pub(super) api_base: &'request str,
    pub(super) api_key: Option<&'request SecretString>,
    pub(super) configured_bearer: Option<&'request SecretString>,
}

pub(super) fn sign(request: Signing<'_>, keys: &Keys) -> Result<AwsSignedRequest, SigningError> {
    let credentials = Credentials {
        access_key_id: SecretString::from(keys.access_key_id),
        secret_access_key: SecretString::from(keys.secret_access_key),
        session_token: keys.session_token.map(SecretString::from),
    };
    let optional_params = InvokeParams::new();
    let now = UNIX_EPOCH + Duration::from_secs(FIXTURE_NOW);
    let runtime = Builder::new_current_thread()
        .build()
        .expect("the test runtime builds");
    runtime.block_on(sign_messages_request(
        MessagesSignRequest {
            headers: request.headers,
            optional_params: &optional_params,
            request_data: request.request_data,
            api_base: request.api_base,
            api_key: request.api_key,
            configured_bearer: request.configured_bearer,
            model: Some(request.model),
            stream: None,
            fake_stream: None,
            region_inputs: RegionInputs::default(),
            now,
        },
        move |_settings, _region| ready(Ok::<_, CredentialError>(credentials)),
    ))
}

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<(Level, String)>>>);

impl Logs {
    pub(super) fn warnings(&self) -> Vec<String> {
        self.messages(Level::WARN)
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
struct Recorded(String);

impl Visit for Recorded {
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
        let mut message = Recorded::default();
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
