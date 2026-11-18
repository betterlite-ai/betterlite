use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::compatible_wire::{NEW_ID, ReplyOptions, system_clock};
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::sagemaker_completion::params::Options;
use betterlite_openai::sagemaker_completion::response::TextTokenCounter;
use betterlite_prompts::arguments::json::Object as Body;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::Hub;
use futures_util::stream;
use http::HeaderMap;
use secrecy::ExposeSecret;
use secrecy::SecretString;

use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::sagemaker_runtime::chat::ChatCall;
use crate::sagemaker_runtime::completion::CompletionCall;
use crate::sagemaker_runtime::embedding::EmbeddingCall;
use crate::sagemaker_runtime::endpoint::ParamMap;
use crate::sagemaker_runtime::runtime::{
    ChunkStream, SagemakerCredentials, SagemakerEnvironment, SagemakerRuntime, SagemakerTransport,
};

struct Counter;

impl TextTokenCounter for Counter {
    fn count_response_tokens(&self, text: &str) -> u64 {
        u64::try_from(text.split_whitespace().count()).unwrap_or_default()
    }
}

struct Encoding;

impl TextEncoding for Encoding {
    fn count_tokens(
        &self,
        _text: &str,
    ) -> Result<usize, betterlite_tokenizer::engine::error::EngineError> {
        Err(betterlite_tokenizer::engine::error::EngineError::PoolClosed)
    }
}

struct FakeCredentials;

impl SagemakerCredentials for FakeCredentials {
    async fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> Result<Credentials, CredentialError> {
        Ok(Credentials {
            access_key_id: SecretString::from("access".to_owned()),
            secret_access_key: SecretString::from("secret".to_owned()),
            session_token: None,
        })
    }
}

struct FakeTransport {
    status: u16,
    body: Vec<u8>,
    chunks: Vec<Vec<u8>>,
    requests: Mutex<Vec<OutboundRequest>>,
}

impl FakeTransport {
    fn new(status: u16, body: &str, chunks: &[&[u8]]) -> Self {
        Self {
            status,
            body: body.as_bytes().to_vec(),
            chunks: chunks.iter().map(|chunk| chunk.to_vec()).collect(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, request: OutboundRequest) {
        self.requests.lock().expect("request lock").push(request);
    }
}

impl SagemakerTransport for FakeTransport {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        self.record(request);
        Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
        })
    }

    async fn open_stream(
        &self,
        request: OutboundRequest,
    ) -> Result<(u16, ChunkStream), TransportError> {
        self.record(request);
        let chunks = self
            .chunks
            .iter()
            .cloned()
            .map(Ok::<Vec<u8>, HttpError>)
            .collect::<Vec<_>>();
        Ok((self.status, Box::pin(stream::iter(chunks))))
    }
}

fn runtime<'counter>(
    transport: FakeTransport,
    counter: &'counter Counter,
) -> SagemakerRuntime<'counter, FakeTransport, FakeCredentials> {
    SagemakerRuntime {
        transport,
        credentials: FakeCredentials,
        environment: SagemakerEnvironment {
            region_name: Some(String::from("us-west-2")),
            region: None,
            session_region: None,
        },
        clock: aws_smithy_async::time::SharedTimeSource::new(
            aws_smithy_async::time::StaticTimeSource::new(UNIX_EPOCH),
        ),
        counter,
        timeout: Duration::from_secs(5),
    }
}

fn prompt_factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(BTreeMap::new()),
        hub: Hub::Unreachable,
    }
}

#[tokio::test]
async fn credentials_are_resolved_for_the_selected_region() {
    let counter = Counter;
    let runtime = runtime(FakeTransport::new(200, "{}", &[]), &counter);
    let credentials = runtime
        .credentials_for(&ParamMap::new(), "eu-west-1")
        .await
        .expect("test credentials");
    assert_eq!(credentials.access_key_id.expose_secret(), "access");
    assert_eq!(credentials.secret_access_key.expose_secret(), "secret");
}

fn completion_call<'call>(
    model: &'call str,
    messages: &'call [Message],
    optional_params: &'call Options,
    signing: &'call ParamMap,
    prompts: &'call PromptFactory,
    custom_prompt_dict: &'call BTreeMap<
        String,
        betterlite_openai::sagemaker_completion::request::CustomPromptEntry,
    >,
    global_custom_prompt_dict: &'call BTreeMap<
        String,
        betterlite_openai::sagemaker_completion::request::CustomPromptEntry,
    >,
) -> CompletionCall<'call> {
    CompletionCall {
        model,
        messages,
        optional_params,
        signing,
        custom_prompt_dict,
        global_custom_prompt_dict,
        hf_model_name: None,
        prompts,
    }
}

fn chat_call<'call>(
    messages: &'call [ChatMessage],
    optional_params: &'call Body,
    signing: &'call ParamMap,
    stream: bool,
) -> ChatCall<'call> {
    ChatCall {
        model: "sagemaker_chat/endpoint",
        messages,
        optional_params,
        signing,
        headers: Headers::new(),
        hf_model_name: None,
        api_key: None,
        configured_bearer: None,
        stream,
        reply: ReplyOptions {
            json_mode: false,
            provider: None,
            base_model: None,
            identifier: NEW_ID,
            clock: system_clock,
        },
    }
}

#[tokio::test]
async fn prepare_calls_build_signed_completion_chat_and_embedding_requests() {
    let counter = Counter;
    let runtime = runtime(FakeTransport::new(200, "{}", &[]), &counter);
    let prompts = prompt_factory();
    let signing = ParamMap::new();
    let custom_prompts = BTreeMap::new();
    let global_prompts = BTreeMap::new();
    let completion_messages: Vec<Message> =
        serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).expect("messages");
    let options = Options::from([(String::from("stream"), serde_json::Value::Bool(true))]);
    let completion = completion_call(
        "sagemaker/endpoint",
        &completion_messages,
        &options,
        &signing,
        &prompts,
        &custom_prompts,
        &global_prompts,
    );
    let prepared = runtime
        .prepare_completion(&completion)
        .await
        .expect("completion request");
    assert!(prepared.url.ends_with("/invocations-response-stream"));
    assert!(
        prepared
            .headers
            .iter()
            .any(|(name, _)| name == "Authorization")
    );
    assert!(prepared.body_json.contains_key("inputs"));

    let chat_messages: Vec<ChatMessage> =
        serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).expect("messages");
    let chat_options: Body = serde_json::from_str(r#"{"model_id":"component"}"#).expect("options");
    let chat = chat_call(&chat_messages, &chat_options, &signing, false);
    let prepared = runtime.prepare_chat(&chat).await.expect("chat request");
    assert!(prepared.headers.iter().any(|(name, value)| {
        name == "X-Amzn-SageMaker-Inference-Component" && value == "component"
    }));
    assert!(
        prepared
            .headers
            .iter()
            .any(|(name, _)| name == "Authorization")
    );

    let input = EmbeddingInput::Text(String::from("hi"));
    let embedding_options = Body::new();
    let encoding = Encoding;
    let embedding = EmbeddingCall {
        model: "voyage-3",
        input: &input,
        optional_params: &embedding_options,
        signing: &signing,
        encoding: &encoding,
    };
    let prepared = runtime
        .prepare_embedding(&embedding)
        .await
        .expect("embedding request");
    assert!(prepared.call.url.ends_with("/invocations"));
    assert!(
        prepared
            .call
            .headers
            .iter()
            .any(|(name, _)| name == "Authorization")
    );
}

#[tokio::test]
async fn chat_region_comes_from_the_model_arn_before_the_environment() {
    let counter = Counter;
    let runtime = runtime(FakeTransport::new(200, "{}", &[]), &counter);
    let messages: Vec<ChatMessage> =
        serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).expect("messages");
    let options = Body::new();
    let signing = ParamMap::new();
    let mut chat = chat_call(&messages, &options, &signing, false);
    chat.model = "arn:aws:sagemaker:eu-central-1:123456789012:endpoint/example";
    let prepared = runtime.prepare_chat(&chat).await.expect("chat request");
    assert_eq!(prepared.region, "eu-central-1");
    assert!(
        prepared
            .url
            .starts_with("https://runtime.sagemaker.eu-central-1.")
    );
}

#[tokio::test]
async fn runtime_calls_preserve_error_status_and_stream_policies() {
    let counter = Counter;
    let prompts = prompt_factory();
    let signing = ParamMap::new();
    let custom_prompts = BTreeMap::new();
    let global_prompts = BTreeMap::new();
    let completion_messages: Vec<Message> =
        serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).expect("messages");
    let completion_options = Options::new();
    let completion = completion_call(
        "sagemaker/endpoint",
        &completion_messages,
        &completion_options,
        &signing,
        &prompts,
        &custom_prompts,
        &global_prompts,
    );
    let error = runtime(FakeTransport::new(429, "throttled", &[]), &counter)
        .completion(&completion)
        .await
        .expect_err("completion status");
    assert_eq!(
        (error.status_code, error.message.as_str()),
        (429, "throttled")
    );

    let chat_messages: Vec<ChatMessage> =
        serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).expect("messages");
    let chat_options: Body = Body::new();
    let chat = chat_call(&chat_messages, &chat_options, &signing, false);
    let error = runtime(FakeTransport::new(429, "chat throttled", &[]), &counter)
        .chat(&chat)
        .await
        .expect_err("chat status");
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "chat throttled");

    let completion_stream_options =
        Options::from([(String::from("stream"), serde_json::Value::Bool(true))]);
    let completion_stream = completion_call(
        "sagemaker/endpoint",
        &completion_messages,
        &completion_stream_options,
        &signing,
        &prompts,
        &custom_prompts,
        &global_prompts,
    );
    let result = runtime(FakeTransport::new(201, "", &[b"failed"]), &counter)
        .completion_stream(&completion_stream)
        .await;
    assert!(matches!(result, Err(error) if error.status_code == 201 && error.message == "failed"));

    let chat_stream = chat_call(&chat_messages, &chat_options, &signing, true);
    let result = runtime(FakeTransport::new(201, "", &[b"failed"]), &counter)
        .chat_stream(&chat_stream)
        .await;
    assert!(matches!(result, Err(error) if error.status_code == 201 && error.message == "failed"));
}
