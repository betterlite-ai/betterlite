use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{Resolve, SystemResolver, UrlValidator};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json as MetadataJson;
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
use betterlite_prompts::bedrock_images::remote::{RemoteImage, TRANSPORT_LIMITS};
use betterlite_prompts::bedrock_processor::error::ProcessorError;
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use betterlite_prompts::message_content::Message;
use indexmap::IndexMap;
use tokio::runtime::{Builder, Runtime};
use tracing::subscriber::{DefaultGuard, set_default};

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::converse_request::error::ConverseRequestError;
use crate::converse_request::{
    ConverseMedia, ConverseRequestInput, ConverseRequestOutcome, ConverseSettings, converse_request,
};
use crate::event_frames::beta::BetaHeader;
use crate::mantle_inputs::region::split_region_prefix;
use crate::model_routes::{ParamMap, ParamValue};
use crate::request_policy::BedrockRequestPolicy;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

pub(super) const CLAUDE_3_5: &str = "anthropic.claude-3-5-test";
pub(super) const CLAUDE_4_5: &str = "anthropic.claude-4-5-test";
pub(super) const ARN: &str =
    "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/profile";
pub(super) const LLAMA: &str = "meta.llama-test";

pub(super) const MODEL_COST: &str = r#"{
    "anthropic.claude-3-5-test": {
        "litellm_provider": "bedrock",
        "supports_prompt_caching": true,
        "supports_native_structured_output": true,
        "supports_output_config": true,
        "supports_adaptive_thinking": false
    },
    "anthropic.claude-4-5-test": {
        "litellm_provider": "bedrock",
        "supports_prompt_caching": true,
        "cache_creation_input_token_cost_above_1hr": 0.0000024,
        "supports_parallel_tool_use_config": true,
        "thinking_always_on": true,
        "supports_adaptive_thinking": false
    },
    "meta.llama-test": {
        "litellm_provider": "bedrock",
        "supports_prompt_caching": false
    }
}"#;

pub(super) const USER_MESSAGES: &str = r#"[{"role": "user", "content": "hi"}]"#;

pub(super) fn params(source: &str) -> ParamMap {
    serde_json::from_str(source).expect("the parameters parse")
}

pub(super) fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("the object parses")
}

pub(super) fn messages(source: &str) -> Vec<Message> {
    serde_json::from_str(source).expect("the messages parse")
}

pub(super) fn value<T: serde::Serialize>(value: &T) -> Json {
    let encoded = serde_json::to_string(value).expect("the value renders");
    serde_json::from_str(&encoded).expect("the rendered value reads back")
}

pub(super) fn expected(source: &str) -> Json {
    serde_json::from_str(source).expect("the expected JSON parses")
}

pub(super) fn field<'value>(value: &'value Json, name: &str) -> Option<&'value Json> {
    value.as_object().and_then(|fields| fields.get(name))
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Settings {
    pub(super) global_drop: bool,
    pub(super) modify_params: bool,
    pub(super) neutralize: bool,
}

pub(super) struct Call<'request> {
    pub(super) model: &'request str,
    pub(super) messages: &'request str,
    pub(super) optional_params: &'request str,
    pub(super) litellm_params: Option<&'request str>,
    pub(super) headers: Option<&'request str>,
    pub(super) model_cost: &'request str,
    pub(super) policy: Option<&'request [&'request str]>,
    pub(super) answers: &'request str,
    pub(super) settings: Settings,
}

impl<'request> Call<'request> {
    pub(super) const fn new(model: &'request str, optional_params: &'request str) -> Self {
        Self {
            model,
            messages: USER_MESSAGES,
            optional_params,
            litellm_params: None,
            headers: None,
            model_cost: MODEL_COST,
            policy: None,
            answers: "{}",
            settings: Settings::default(),
        }
    }
}

pub(super) struct Outcome {
    pub(super) body: Result<Json, ConverseRequestError>,
    pub(super) optional_params: Json,
    pub(super) litellm_params: Option<Json>,
    pub(super) tool_name_mappings: ToolNameMap,
}

impl Outcome {
    pub(super) fn body(&self) -> &Json {
        self.body
            .as_ref()
            .unwrap_or_else(|error| panic!("the request should assemble: {error}"))
    }

    pub(super) fn error(&self) -> &ConverseRequestError {
        match &self.body {
            Ok(value) => panic!("the request should fail, got {value:?}"),
            Err(error) => error,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the model is not mapped")]
pub(super) struct Unmapped;

pub(super) struct Answers(BTreeMap<String, IndexMap<String, MetadataJson>>);

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

pub(super) struct NoHfTransport;

#[derive(Debug, thiserror::Error)]
#[error("the metadata lookup must not fetch a Hugging Face config")]
pub(super) struct NoHfError;

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
    provider_catalog: Catalog,
    hf: HfConfigClient<NoHfTransport>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    patterns: CandidatePatterns,
    runtime: RawCatalog,
    unlisted: RawCatalog,
    packaged: Arc<ModelCatalog>,
}

impl Fixture {
    pub(super) fn load(model_cost: &ParamMap) -> Self {
        let bytes = serde_json::to_vec(model_cost).expect("the model cost map renders");
        Self {
            entries: ModelCostMap::parse(&bytes).expect("the model cost map parses"),
            typed: ModelCatalog::parse(b"{}", UnknownFields::Warn)
                .expect("an empty catalog parses"),
            provider_catalog: Catalog::new(),
            hf: HfConfigClient {
                client: NoHfTransport,
                base_url: HF_CONFIG_BASE_URL.to_owned(),
            },
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            patterns: CandidatePatterns::new().expect("the candidate patterns compile"),
            runtime: serde_json::from_slice(&bytes).expect("the raw catalog parses"),
            unlisted: RawCatalog::new(),
            packaged: BundledCatalog::default().get(),
        }
    }

    pub(super) const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime,
            packaged: &self.unlisted,
        }
    }

    pub(super) fn packaged(&self) -> &ModelCatalog {
        self.packaged.as_ref()
    }

    fn environment<'fixture>(
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
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime builds")
}

#[derive(Clone, Default)]
pub(super) struct Warnings(Arc<Mutex<Vec<u8>>>);

impl Warnings {
    pub(super) fn install(&self) -> DefaultGuard {
        let writer = self.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::WARN)
            .with_ansi(false)
            .without_time()
            .with_writer(move || writer.clone())
            .finish();
        set_default(subscriber)
    }

    pub(super) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("the warnings are UTF-8")
    }
}

impl io::Write for Warnings {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn run<R: Resolve>(call: &Call<'_>, remote: &RemoteImage<R>) -> (Outcome, String) {
    run_on(&runtime(), call, remote)
}

struct Session<'setup, 'request, R> {
    call: &'setup Call<'request>,
    remote: &'setup RemoteImage<R>,
    model_cost: ParamMap,
    fixture: Fixture,
    optional_params: ParamMap,
    litellm_params: Option<Object>,
    headers: BTreeMap<String, BetaHeader>,
    parsed_messages: Vec<Message>,
    policy: Option<BedrockRequestPolicy>,
    answers: Answers,
}

impl<'setup, 'request, R: Resolve> Session<'setup, 'request, R> {
    fn new(call: &'setup Call<'request>, remote: &'setup RemoteImage<R>) -> Self {
        let model_cost = params(call.model_cost);
        Self {
            call,
            remote,
            optional_params: params(call.optional_params),
            litellm_params: call.litellm_params.map(object),
            headers: call
                .headers
                .map(|source| serde_json::from_str(source).expect("the headers parse"))
                .unwrap_or_default(),
            parsed_messages: messages(call.messages),
            policy: call.policy.map(|fields| BedrockRequestPolicy {
                fields: fields.iter().map(|field| (*field).to_owned()).collect(),
            }),
            answers: Answers(serde_json::from_str(call.answers).expect("the lookup answers parse")),
            fixture: Fixture::load(&model_cost),
            model_cost,
        }
    }

    async fn assemble(&mut self) -> Result<ConverseRequestOutcome, ConverseRequestError> {
        let catalogs = self.fixture.catalogs();
        let mut keys = ModelCostKeys::default();
        let dynamic = NoDynamicMetadata;
        let resolution = self.fixture.resolution();
        let mut environment = self.fixture.environment(&resolution, &mut keys, &dynamic);
        let new_id = || "call_generated".to_owned();
        let input = ConverseRequestInput {
            model: self.call.model,
            messages: &self.parsed_messages,
            optional_params: &mut self.optional_params,
            litellm_params: self.litellm_params.as_mut(),
            headers: Some(&self.headers),
            settings: ConverseSettings {
                global_drop: self.call.settings.global_drop,
                modify_params: self.call.settings.modify_params,
                neutralize_orphans: self.call.settings.neutralize,
                request_metadata: self.policy.as_ref(),
            },
            model_cost: &self.model_cost,
            catalogs: &catalogs,
            lookup: &self.answers,
            packaged: self.fixture.packaged(),
        };
        let media = ConverseMedia {
            remote: self.remote,
            new_id: &new_id,
        };
        converse_request(input, media, &mut environment).await
    }
}

pub(super) fn run_on<R: Resolve>(
    runtime: &Runtime,
    call: &Call<'_>,
    remote: &RemoteImage<R>,
) -> (Outcome, String) {
    let mut session = Session::new(call, remote);
    let warnings = Warnings::default();
    let assembled = {
        let _captured = warnings.install();
        runtime.block_on(session.assemble())
    };
    let (body, tool_name_mappings) = match assembled {
        Ok(outcome) => (Ok(value(&outcome.body)), outcome.tool_name_mappings),
        Err(error) => (Err(error), ToolNameMap::new()),
    };
    let outcome = Outcome {
        body,
        optional_params: value(&session.optional_params),
        litellm_params: session.litellm_params.as_ref().map(value),
        tool_name_mappings,
    };
    (outcome, warnings.text())
}

pub(super) fn processor_error(error: &ConverseRequestError) -> Option<&ProcessorError> {
    match error {
        ConverseRequestError::Processor(source) => Some(source),
        _ => None,
    }
}

pub(super) fn remote(allowed: &[&str]) -> RemoteImage<SystemResolver> {
    remote_for(SystemResolver, allowed, 50.0)
}

pub(super) fn remote_for<R: Resolve>(resolver: R, allowed: &[&str], max_mb: f64) -> RemoteImage<R> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    let client = ProxiedClient::build(base_builder(), &policy, TRANSPORT_LIMITS)
        .expect("the proxied client builds");
    let hosts: Vec<String> = allowed.iter().map(|host| (*host).to_owned()).collect();
    let validator = UrlValidator::new(resolver, &hosts, &Verification::Enabled);
    RemoteImage {
        client,
        urls: UserUrlFetcher {
            validator,
            validation: true,
        },
        deadline: TRANSPORT_LIMITS.read_timeout,
        max_download_size_mb: max_mb,
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Refusing;

#[async_trait::async_trait]
impl Resolve for Refusing {
    async fn resolve(&self, _host: &str, _port: u16) -> io::Result<Vec<SocketAddr>> {
        Err(io::Error::other("the resolver refuses every host"))
    }
}
