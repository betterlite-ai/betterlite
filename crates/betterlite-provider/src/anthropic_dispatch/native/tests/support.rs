use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::anthropic_http_stream::call::{StreamLogger, StreamPostCall};
use betterlite_anthropic::batches::{
    config::AnthropicBatchesConfig, handler::AnthropicBatchesHandler, timestamps::Clock,
};
use betterlite_anthropic::count_tokens::handler::AnthropicCountTokensHandler;
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_anthropic::files::{api::FilesApi, request::FilesClient};
use betterlite_anthropic::option_primitives::OptionDefaults;
use betterlite_anthropic::request::BillingHeader;
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::stream_state::state::UuidV4;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::{
    MetadataEnvironment,
    dynamic::NoDynamicMetadata,
    hf_config::HfConfigClient,
    names::{ModelCostMap, NameNormalizers},
};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_prompts::anthropic::assembly::PromptDeps;
use betterlite_prompts::anthropic_results::tool_result::FileBlocks;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::factory::{PromptFactory, hub::Hub};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::engine::{
    Engine,
    assets::bundled,
    tiktoken::{Encoding, TiktokenEngine},
};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Value;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::timeout;

use super::super::{Failure, Native, Reply, Runtime, legacy::Legacy, preparation::Preparation};
use crate::anthropic_dispatch::{
    CallLogging, DispatchError,
    base::{Environment, Globals, Surface},
    completion::{CompletionCall, CompletionSettings},
    dispatch,
    operations::{DispatchDeps, Operation, Outcome},
};

const TIMEOUT: Duration = Duration::from_secs(5);
const COSTS: &[u8] = br#"{"claude-3-haiku-20240307":{"litellm_provider":"anthropic","supports_function_calling":true}}"#;

pub(super) struct Observed {
    pub posts: usize,
    pub content: Option<String>,
    pub body: Value,
    pub key: String,
    pub status: Option<u16>,
    pub logged: String,
}

struct Media;
impl RemoteMediaFetcher for Media {
    async fn convert_url(&self, _url: &str) -> Result<String, ImageFetchError> {
        Ok("data:image/png;base64,aGk=".to_owned())
    }
    async fn convert_urls(&self, urls: &[String]) -> Result<Vec<String>, ImageFetchError> {
        Ok(vec!["data:image/png;base64,aGk=".to_owned(); urls.len()])
    }
}
impl FileBlocks for Media {
    type Failure = std::convert::Infallible;
    fn file_block(&self, element: &Object) -> Result<Object, Self::Failure> {
        Ok(element.clone())
    }
}
struct Log;
impl StreamLogger for Log {
    fn post_call(&self, _call: StreamPostCall<'_>) {}
}

async fn server(listener: TcpListener, answer: &'static str) -> (Value, String) {
    let (mut socket, _) = listener.accept().await.unwrap();
    let mut input = Vec::new();
    let (end, length) = loop {
        let mut buf = [0; 4096];
        let count = socket.read(&mut buf).await.unwrap();
        assert_ne!(count, 0);
        input.extend_from_slice(&buf[..count]);
        if let Some(end) = input.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            let headers = str::from_utf8(&input[..end]).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            break (end + 4, length);
        }
        assert!(input.len() < 16384);
    };
    while input.len() < end + length {
        let mut buf = [0; 4096];
        let count = socket.read(&mut buf).await.unwrap();
        assert_ne!(count, 0);
        input.extend_from_slice(&buf[..count]);
    }
    let headers = str::from_utf8(&input[..end]).unwrap();
    let key = headers
        .lines()
        .find_map(|line| {
            line.strip_prefix("x-api-key:")
                .map(|value| value.trim().to_owned())
        })
        .unwrap();
    let body = serde_json::from_slice(&input[end..end + length]).unwrap();
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
        answer.len()
    );
    socket.write_all(reply.as_bytes()).await.unwrap();
    (body, key)
}

pub(super) async fn run(
    surface: Surface,
    asynchronous: bool,
    key: bool,
    tag: ChatConfigTag,
) -> Observed {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut tasks = JoinSet::new();
    let answer = match surface {
        Surface::Messages => {
            r#"{"id":"msg_native","type":"message","role":"assistant","model":"claude-3-haiku-20240307","content":[{"type":"text","text":"native answer"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":2}}"#
        }
        Surface::Legacy => r#"{"completion":"legacy answer","stop_reason":"stop_sequence"}"#,
    };
    tasks.spawn(server(listener, answer));
    let client = ProxiedClient::outbound(&Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap(),
    ))
    .unwrap();
    let map = ModelCostMap::parse(COSTS).unwrap();
    let catalog = ModelCatalog::parse(COSTS, UnknownFields::Warn).unwrap();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &BTreeSet::new(),
        custom_providers: &BTreeSet::new(),
        environment: &BTreeMap::new(),
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let mut keys = ModelCostKeys::default();
    let hf = HfConfigClient {
        client: client.clone(),
        base_url: base.clone(),
    };
    let environment = CapabilityEnvironment {
        metadata: MetadataEnvironment {
            model_cost: &map.entries,
            vertex: &map.vertex,
            generalizations: &CatalogGeneralizations::default(),
            resolution: &resolution,
            normalizers: NameNormalizers {
                bedrock_base_model: get_bedrock_base_model,
                bedrock_routing_prefix: strip_bedrock_routing_prefix,
                mantle_region_split: betterlite_bedrock::mantle_inputs::region::split_region_prefix,
            },
        },
        keys: &mut keys,
        dynamic: &NoDynamicMetadata,
        hf: &hf,
        provider_catalog: &Catalog::new(),
        provider_generation: 0,
    };
    let patterns = CandidatePatterns::new().unwrap();
    let source = serde_json::from_slice(COSTS).unwrap();
    let catalogs = CatalogSources {
        patterns: &patterns,
        runtime: &source,
        packaged: &source,
    };
    let defaults: OptionDefaults = serde_json::from_str(r#"{"max_tokens":17}"#).unwrap();
    let native_settings = AnthropicSettings::default();
    let matrix = serde_json::from_str(r#"{"anthropic":{}}"#).unwrap();
    let prompts = PromptFactory {
        cache: Mutex::new(BTreeMap::new()),
        hub: Hub::Unreachable,
    };
    let mut handler = Native {
        runtime: Runtime {
            pooled: &client,
            settings: &native_settings,
            matrix: &matrix,
            stream_logger: &Log,
            ids: &UuidV4,
            now: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            max_event_bytes: 65536,
            max_error_body_bytes: 65536,
            disable_add_prefix_to_prompt: false,
            effort_used: false,
        },
        preparation: Preparation {
            environment,
            catalogs,
            prompts: PromptDeps {
                fetcher: &Media,
                blocks: &Media,
                new_id: &|| uuid::Uuid::new_v4().to_string(),
            },
            catalog: &catalog,
            defaults: &defaults,
            max_tokens: 17,
            billing: BillingHeader::Keep,
            modify_params: false,
            drop_params: false,
        },
        legacy: Legacy {
            prompts: &prompts,
            defaults: &betterlite_config::sections::anthropic_completion::OptionValues::default(),
            body_limit: 65536,
        },
    };
    let encoding = Encoding::Cl100kBase;
    let engine = Engine::Tiktoken(Arc::new(
        TiktokenEngine::from_bpe_file(encoding, &bundled(encoding).unwrap()).unwrap(),
    ));
    let call = CompletionCall {
        model: match surface {
            Surface::Messages => "claude-3-haiku-20240307",
            Surface::Legacy => "claude-2",
        }
        .to_owned(),
        provider: "anthropic".to_owned(),
        messages: if key {
            serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).unwrap()
        } else {
            vec![betterlite_models::decisions::Json::Bool(true)]
        },
        model_response: ResponseEnvelope::new("chatcmpl-native".to_owned()),
        optional_params: IndexMap::new(),
        litellm_params: IndexMap::new(),
        headers: BTreeMap::new(),
        api_key: key.then(|| SecretString::from("native-key")),
        api_base: Some(base),
        custom_prompt_dict: BTreeMap::new(),
        stream: false,
        acompletion: asynchronous,
        client: None,
        shared_session: None,
        encoding: engine,
        timeout: TIMEOUT,
    };
    let globals = Globals::default();
    let environment = Environment::default();
    let completion = CompletionSettings {
        globals: &globals,
        environment: &environment,
        custom_prompt_dict: &BTreeMap::new(),
        disable_suffix: false,
        selection: Some(tag),
    };
    let count = AnthropicCountTokensHandler {
        transport: client.clone(),
        request_timeout: TIMEOUT,
        beta_version: "token-counting-2024-11-01".to_owned(),
    };
    let batches = AnthropicBatchesHandler {
        transport: client.clone(),
        config: AnthropicBatchesConfig {
            settings: native_settings.clone(),
        },
        clock: Clock {
            now: 1_700_000_000,
            zone: FixedOffset::default(),
        },
        request_timeout: TIMEOUT,
    };
    let files = FilesApi {
        transport: client.clone(),
        clock: || 1_700_000_000,
    };
    let file_client = FilesClient {
        api_key: None,
        api_base: None,
        settings: native_settings.clone(),
        timeout: TIMEOUT,
    };
    let mut deps = DispatchDeps {
        completion: &completion,
        handler: &mut handler,
        count: &count,
        batches: &batches,
        files: &files,
        file_client: &file_client,
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let (mut state, env) =
        crate::anthropic_dispatch::fixtures::runtime::logging(&redactor).unwrap();
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let outcome = timeout(
        TIMEOUT,
        dispatch(
            Operation::Completion(surface, Box::new(call)),
            &mut deps,
            &mut log,
        ),
    )
    .await
    .unwrap();
    let (content, status, posted) = match outcome {
        Ok(Outcome::Completion(Reply::Messages(
            betterlite_anthropic::chat_call::ChatOutcome::Reply(reply),
        ))) => (reply.choices[0].message.content.clone(), None, true),
        Ok(Outcome::Completion(Reply::Legacy(super::super::legacy::Reply::Buffered(reply)))) => {
            (reply.choices[0].message.content.clone(), None, true)
        }
        Err(DispatchError::Completion(Failure::Chat(error))) => (None, error.status_code(), false),
        Err(DispatchError::Completion(Failure::Config)) => (None, None, false),
        _ => panic!("unexpected native outcome"),
    };
    let logged = match state.model_call_details.get("original_response") {
        Some(betterlite_models::decisions::Json::Text(text)) => text.clone(),
        _ => String::new(),
    };
    let observed = if posted {
        let (body, key) = timeout(TIMEOUT, tasks.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        Observed {
            posts: 1,
            content,
            body,
            key,
            status,
            logged,
        }
    } else {
        Observed {
            posts: 0,
            content,
            body: Value::Null,
            key: String::new(),
            status,
            logged,
        }
    };
    client.drain(TIMEOUT).await.unwrap();
    observed
}
