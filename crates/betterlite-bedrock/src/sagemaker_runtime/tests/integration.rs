use aws_smithy_async::time::{SharedTimeSource, StaticTimeSource};
use betterlite_tokenizer::engine::error::EngineError;
use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};
use tokio::net::TcpListener;

use aws_credential_types::provider::error::CredentialsError;
use aws_smithy_eventstream::frame::write_message_to;
use aws_smithy_types::event_stream::{Header, HeaderValue, Message as EventMessage};
use aws_smithy_types::str_bytes::StrBytes;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::{proxy::ProxyPolicy, types::ProxyTrust};
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::compatible_wire::{NEW_ID, ReplyOptions, system_clock};
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_openai::sagemaker_completion::params::Options;
use betterlite_openai::sagemaker_completion::request::CustomPromptEntry;
use betterlite_openai::sagemaker_completion::response::TextTokenCounter;
use betterlite_prompts::arguments::json::Object as Body;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::Hub;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sigv4::sign::{Signable, SigningMode, sign_request};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::{HeaderMap, Method};
use rstest::rstest;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use serde_json::json;
use url::Url;
use uuid::Uuid;

use crate::aws_settings::sources::{Credentials, SourceError};
use crate::credentials::error::CredentialError;
use crate::sagemaker_runtime::chat::{self, ChatCall, ChatVariant};
use crate::sagemaker_runtime::completion::{self, CompletionCall};
use crate::sagemaker_runtime::embedding::{self, EmbeddingCall, EmbeddingReply};
use crate::sagemaker_runtime::endpoint::ParamMap;
use crate::sagemaker_runtime::error::RuntimeFailure;
use crate::sagemaker_runtime::runtime::{
    ChunkStream, SagemakerCredentials, SagemakerEnvironment, SagemakerRuntime, SagemakerTransport,
};
use crate::sagemaker_stream::SagemakerChunk;
use crate::signing::headers::{filter_for_signature, without_computed};

const IDENTIFIER: u128 = 7;

struct Counter;

impl TextTokenCounter for Counter {
    fn count_response_tokens(&self, text: &str) -> u64 {
        u64::try_from(text.split_whitespace().count()).unwrap_or_default()
    }
}

struct Encoding;

impl TextEncoding for Encoding {
    fn count_tokens(&self, _text: &str) -> Result<usize, EngineError> {
        Err(EngineError::PoolClosed)
    }
}

fn credentials() -> Credentials {
    Credentials {
        access_key_id: SecretString::from("access".to_owned()),
        secret_access_key: SecretString::from("secret".to_owned()),
        session_token: None,
    }
}

struct TestCredentials {
    failure: Mutex<Option<CredentialError>>,
    regions: Mutex<Vec<String>>,
}

impl TestCredentials {
    fn working() -> Self {
        Self {
            failure: Mutex::new(None),
            regions: Mutex::new(Vec::new()),
        }
    }

    fn failing(failure: CredentialError) -> Self {
        Self {
            failure: Mutex::new(Some(failure)),
            regions: Mutex::new(Vec::new()),
        }
    }
}

impl SagemakerCredentials for TestCredentials {
    fn resolve(
        &self,
        _settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.regions.lock().expect("regions lock").push(region);
        let failure = self.failure.lock().expect("failure lock").take();
        ready(failure.map_or_else(|| Ok(credentials()), Err))
    }
}

struct FakeTransport {
    status: u16,
    body: Vec<u8>,
    chunks: Vec<Vec<u8>>,
    failure: Mutex<Option<TransportError>>,
    requests: Mutex<Vec<OutboundRequest>>,
}

impl FakeTransport {
    fn new(status: u16, body: &str, chunks: &[&[u8]]) -> Self {
        Self {
            status,
            body: body.as_bytes().to_vec(),
            chunks: chunks.iter().map(|chunk| chunk.to_vec()).collect(),
            failure: Mutex::new(None),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn failure(error: TransportError) -> Self {
        Self {
            failure: Mutex::new(Some(error)),
            ..Self::new(0, "", &[])
        }
    }

    fn reply(&self, request: OutboundRequest) -> Result<(u16, Vec<Vec<u8>>), TransportError> {
        let failure = self
            .failure
            .lock()
            .map_err(|_poisoned| TransportError::Closed)?
            .take();
        if let Some(error) = failure {
            return Err(error);
        }
        self.requests
            .lock()
            .map_err(|_poisoned| TransportError::Closed)?
            .push(request);
        Ok((self.status, self.chunks.clone()))
    }
}

impl SagemakerTransport for FakeTransport {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(
            self.reply(request)
                .map(|(status, _chunks)| UpstreamResponse {
                    status,
                    headers: HeaderMap::new(),
                    body: self.body.clone(),
                }),
        )
    }

    fn open_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<(u16, ChunkStream), TransportError>> + Send {
        let (status, chunks) = match self.reply(request) {
            Ok(reply) => reply,
            Err(error) => return ready(Err(error)),
        };
        let items = chunks
            .into_iter()
            .map(Ok::<Vec<u8>, HttpError>)
            .collect::<Vec<_>>();
        let body: ChunkStream = Box::pin(stream::iter(items));
        ready(Ok((status, body)))
    }
}

fn fixed_identifier() -> Uuid {
    Uuid::from_u128(IDENTIFIER)
}

fn runtime<T, C>(transport: T, credentials: C, counter: &Counter) -> SagemakerRuntime<'_, T, C>
where
    T: SagemakerTransport,
    C: SagemakerCredentials,
{
    SagemakerRuntime {
        transport,
        credentials,
        environment: SagemakerEnvironment {
            region_name: Some(String::from("us-west-2")),
            region: None,
            session_region: None,
        },
        clock: SharedTimeSource::new(StaticTimeSource::new(UNIX_EPOCH)),
        counter,
        timeout: Duration::from_secs(5),
        identifier: fixed_identifier,
    }
}

fn prompt_factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(BTreeMap::new()),
        hub: Hub::Unreachable,
    }
}

fn prompt_messages() -> Vec<Message> {
    serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).expect("prompt messages")
}

fn chat_messages() -> Vec<ChatMessage> {
    serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).expect("chat messages")
}

struct Workspace {
    prompts: PromptFactory,
    signing: ParamMap,
    custom_prompts: BTreeMap<String, CustomPromptEntry>,
    global_prompts: BTreeMap<String, CustomPromptEntry>,
    messages: Vec<Message>,
    chat_messages: Vec<ChatMessage>,
    chat_options: Body,
    options: Options,
    embedding_input: EmbeddingInput,
    embedding_options: Body,
    encoding: Encoding,
}

impl Workspace {
    fn new() -> Self {
        Self {
            prompts: prompt_factory(),
            signing: ParamMap::new(),
            custom_prompts: BTreeMap::new(),
            global_prompts: BTreeMap::new(),
            messages: prompt_messages(),
            chat_messages: chat_messages(),
            chat_options: Body::new(),
            options: Options::new(),
            embedding_input: EmbeddingInput::Text(String::from("hi")),
            embedding_options: Body::new(),
            encoding: Encoding,
        }
    }

    fn completion(&self) -> CompletionCall<'_> {
        CompletionCall {
            model: "sagemaker/endpoint",
            messages: &self.messages,
            optional_params: &self.options,
            signing: &self.signing,
            custom_prompt_dict: &self.custom_prompts,
            global_custom_prompt_dict: &self.global_prompts,
            hf_model_name: None,
            prompts: &self.prompts,
        }
    }

    fn chat(&self, stream: bool) -> ChatCall<'_> {
        ChatCall {
            model: "sagemaker_chat/endpoint",
            messages: &self.chat_messages,
            optional_params: &self.chat_options,
            signing: &self.signing,
            headers: Headers::new(),
            hf_model_name: None,
            variant: ChatVariant::Chat,
            stream,
            reply: ReplyOptions {
                json_mode: false,
                provider: None,
                base_model: None,
                identifier: NEW_ID,
                clock: system_clock,
            },
            configured_bearer: None,
        }
    }

    fn embedding<'call>(&'call self, model: &'call str) -> EmbeddingCall<'call> {
        EmbeddingCall {
            model,
            input: &self.embedding_input,
            optional_params: &self.embedding_options,
            signing: &self.signing,
            encoding: &self.encoding,
        }
    }
}

fn signed_with_sigv4(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .any(|(name, value)| name == "Authorization" && value.starts_with("AWS4-HMAC-SHA256"))
}

fn text_header(value: &str) -> HeaderValue {
    HeaderValue::String(StrBytes::from(value.to_owned()))
}

fn frame(headers: &[(&str, HeaderValue)], payload: &[u8]) -> Vec<u8> {
    let parts = headers
        .iter()
        .map(|(name, value)| Header::new((*name).to_owned(), value.clone()))
        .collect();
    let message = EventMessage::new_from_parts(parts, payload.to_vec());
    let mut out = Vec::new();
    write_message_to(&message, &mut out).expect("frame");
    out
}

fn upstream(failure: RuntimeFailure) -> SagemakerError {
    match failure {
        RuntimeFailure::Sagemaker(error) => error,
        RuntimeFailure::Reply(error) => panic!("a wire error is expected: {error}"),
    }
}

// the class B239 gives the failure the runtime reports, read through the same RawException the dispatch takes
fn classify(failure: &RuntimeFailure) -> ErrorClass {
    let raw = RawException {
        model: String::from("sagemaker/endpoint"),
        custom_llm_provider: String::from("sagemaker"),
        error_str: failure.to_string(),
        status_code: failure.status_code(),
        status_code_is_synthesized: false,
        message: None,
        body: None,
        exception_type: String::new(),
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .expect("built-in redaction patterns always compile");
    dispatch(&DispatchInput::Raw(Box::new(raw)), &redactor)
}

async fn timeout_failure() -> TransportError {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let address = listener.local_addr().expect("listener address");
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .expect("default proxy policy");
    let client = ProxiedClient::outbound(&Arc::new(policy)).expect("timeout client");
    let request = OutboundRequest {
        method: Method::GET,
        url: Url::parse(&format!("http://{address}/invocations")).expect("loopback URL"),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_millis(100),
    };
    let failure = client
        .send_buffered(request)
        .await
        .expect_err("the listener never answers");
    assert!(matches!(failure, TransportError::Deadline(_)));
    failure
}

#[tokio::test]
async fn credentials_are_resolved_for_the_selected_region() {
    let counter = Counter;
    let runtime = runtime(
        FakeTransport::new(200, "{}", &[]),
        TestCredentials::working(),
        &counter,
    );
    let credentials = runtime
        .credentials_for(&ParamMap::new(), "eu-west-1")
        .await
        .expect("test credentials");
    assert_eq!(credentials.access_key_id.expose_secret(), "access");
    assert_eq!(credentials.secret_access_key.expose_secret(), "secret");
    assert_eq!(
        *runtime.credentials.regions.lock().expect("regions lock"),
        ["eu-west-1"]
    );
}

#[tokio::test]
async fn prepare_calls_build_sigv4_signed_completion_chat_and_embedding_requests() {
    let counter = Counter;
    let mut workspace = Workspace::new();
    workspace.options = Options::from([(String::from("stream"), json!(true))]);
    workspace.chat_options =
        serde_json::from_str(r#"{"model_id":"component"}"#).expect("chat options");
    let runtime = runtime(
        FakeTransport::new(200, "{}", &[]),
        TestCredentials::working(),
        &counter,
    );
    let completion_call = completion::prepare_completion(&runtime, &workspace.completion())
        .await
        .expect("completion request");
    assert!(
        completion_call
            .url
            .ends_with("/invocations-response-stream")
    );
    assert!(signed_with_sigv4(&completion_call.headers));
    assert!(completion_call.body_json.contains_key("inputs"));

    let chat_call = chat::prepare_chat(&runtime, &workspace.chat(false))
        .await
        .expect("chat request");
    assert!(chat_call.headers.iter().any(|(name, value)| {
        name == "X-Amzn-SageMaker-Inference-Component" && value == "component"
    }));
    assert!(signed_with_sigv4(&chat_call.headers));

    let prepared = embedding::prepare_embedding(&runtime, &workspace.embedding("voyage-3"))
        .await
        .expect("embedding request");
    assert!(prepared.call.url.ends_with("/invocations"));
    assert!(signed_with_sigv4(&prepared.call.headers));
}

#[rstest]
#[case::completion(false)]
#[case::chat(true)]
#[tokio::test]
async fn transmitted_sigv4_signature_covers_the_sent_request(#[case] chat: bool) {
    let counter = Counter;
    let workspace = Workspace::new();
    let reply = if chat {
        r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"hi"}}]}"#
    } else {
        r#"{"generated_text":"hello"}"#
    };
    let runtime = runtime(
        FakeTransport::new(200, reply, &[]),
        TestCredentials::working(),
        &counter,
    );
    if chat {
        chat::chat(&runtime, &workspace.chat(false))
            .await
            .expect("chat reply");
    } else {
        completion::completion(&runtime, &workspace.completion())
            .await
            .expect("completion reply");
    }
    let mut requests = runtime.transport.requests.lock().expect("request lock");
    let sent = requests.pop().expect("the runtime sent one request");
    drop(requests);
    assert_transmitted_signature(&sent);
}

#[tokio::test]
async fn completion_and_embedding_take_aws_region_before_aws_region_name() {
    let counter = Counter;
    let workspace = Workspace::new();
    let mut runtime = runtime(
        FakeTransport::new(200, "{}", &[]),
        TestCredentials::working(),
        &counter,
    );
    runtime.environment = SagemakerEnvironment {
        region_name: Some(String::from("eu-west-1")),
        region: Some(String::from("us-east-1")),
        session_region: Some(String::from("ap-south-1")),
    };
    let completion = completion::prepare_completion(&runtime, &workspace.completion())
        .await
        .expect("completion request");
    assert_eq!(completion.region, "us-east-1");
    let embedding = embedding::prepare_embedding(&runtime, &workspace.embedding("voyage-3"))
        .await
        .expect("embedding request");
    assert_eq!(embedding.call.region, "us-east-1");
}

#[rstest]
#[case::bedrock_arn(
    "arn:aws:bedrock:eu-central-1:123456789012:model/example",
    "eu-central-1"
)]
#[case::sagemaker_arn(
    "arn:aws:sagemaker:eu-central-1:123456789012:endpoint/example",
    "us-east-2"
)]
#[case::endpoint("endpoint", "us-east-2")]
#[tokio::test]
async fn chat_region_recognizes_only_bedrock_arns(#[case] model: &str, #[case] region: &str) {
    let counter = Counter;
    let workspace = Workspace::new();
    let mut runtime = runtime(
        FakeTransport::new(200, "{}", &[]),
        TestCredentials::working(),
        &counter,
    );
    runtime.environment.region_name = Some(String::from("us-east-2"));
    let mut chat = workspace.chat(false);
    chat.model = model;
    let prepared = chat::prepare_chat(&runtime, &chat)
        .await
        .expect("chat request");
    assert_eq!(prepared.region, region);
    assert!(
        prepared
            .url
            .starts_with(&format!("https://runtime.sagemaker.{region}."))
    );
}

#[tokio::test]
async fn sagemaker_nova_chat_drops_the_model_field() {
    let counter = Counter;
    let workspace = Workspace::new();
    let runtime = runtime(
        FakeTransport::new(200, "{}", &[]),
        TestCredentials::working(),
        &counter,
    );
    let mut chat = workspace.chat(false);
    chat.variant = ChatVariant::Chat;
    let chat_prepared = chat::prepare_chat(&runtime, &chat)
        .await
        .expect("chat request");
    assert!(chat_prepared.body_json.contains_key("model"));
    let mut nova_call = workspace.chat(false);
    nova_call.variant = ChatVariant::Nova;
    let nova = chat::prepare_chat(&runtime, &nova_call)
        .await
        .expect("nova request");
    assert!(!nova.body_json.contains_key("model"));
    assert_eq!(
        nova.body_json.get("messages"),
        chat_prepared.body_json.get("messages")
    );
    assert!(signed_with_sigv4(&nova.headers));
}

#[tokio::test]
async fn completion_and_chat_honor_sagemaker_base_url() {
    let counter = Counter;
    let mut workspace = Workspace::new();
    workspace.options = Options::from([(
        String::from("sagemaker_base_url"),
        json!("https://custom.example/completion"),
    )]);
    workspace.chat_options =
        serde_json::from_str(r#"{"sagemaker_base_url":"https://custom.example/chat"}"#)
            .expect("chat options");
    let runtime = runtime(
        FakeTransport::new(200, "{}", &[]),
        TestCredentials::working(),
        &counter,
    );
    let completion = completion::prepare_completion(&runtime, &workspace.completion())
        .await
        .expect("completion request");
    assert_eq!(completion.url, "https://custom.example/completion");
    let chat = chat::prepare_chat(&runtime, &workspace.chat(false))
        .await
        .expect("chat request");
    assert_eq!(chat.url, "https://custom.example/chat");
}

#[tokio::test]
async fn completion_component_header_is_added_after_signing() {
    let counter = Counter;
    let mut workspace = Workspace::new();
    workspace.options = Options::from([(String::from("model_id"), json!("component"))]);
    let runtime = runtime(
        FakeTransport::new(200, "{}", &[]),
        TestCredentials::working(),
        &counter,
    );
    let prepared = completion::prepare_completion(&runtime, &workspace.completion())
        .await
        .expect("completion request");
    assert!(prepared.headers.iter().any(|(name, value)| {
        name == "X-Amzn-SageMaker-Inference-Component" && value == "component"
    }));
    let authorization = prepared
        .headers
        .iter()
        .find_map(|(name, value)| (name == "Authorization").then_some(value))
        .expect("signed request");
    assert!(
        !authorization
            .to_ascii_lowercase()
            .contains("x-amzn-sagemaker-inference-component")
    );
    let plain_workspace = Workspace::new();
    let plain = completion::prepare_completion(&runtime, &plain_workspace.completion())
        .await
        .expect("plain request");
    assert!(
        !plain
            .headers
            .iter()
            .any(|(name, _)| name == "X-Amzn-SageMaker-Inference-Component")
    );
}

#[tokio::test]
async fn chat_signing_uses_the_configured_bearer_before_resolving_credentials() {
    let counter = Counter;
    let workspace = Workspace::new();
    let bearer = SecretString::from("test-bearer-token".to_owned());
    let mut call = workspace.chat(false);
    call.configured_bearer = Some(&bearer);
    let prepared = chat::prepare_chat(
        &runtime(
            FakeTransport::new(200, "{}", &[]),
            TestCredentials::failing(CredentialError::MissingCredentials),
            &counter,
        ),
        &call,
    )
    .await
    .expect("the bearer skips credential resolution");
    assert!(
        prepared
            .headers
            .iter()
            .any(|(name, value)| name == "Authorization" && value == "Bearer test-bearer-token")
    );
    assert!(!signed_with_sigv4(&prepared.headers));
}

#[tokio::test]
async fn chat_reads_a_compatible_reply() {
    let counter = Counter;
    let workspace = Workspace::new();
    let reply = chat::chat(
        &runtime(
            FakeTransport::new(
                200,
                r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"hi"}}]}"#,
                &[],
            ),
            TestCredentials::working(),
            &counter,
        ),
        &workspace.chat(false),
    )
    .await
    .expect("chat reply");
    assert_eq!(
        reply.response.choices[0].message.content.as_deref(),
        Some("hi")
    );
}

#[tokio::test]
async fn chat_rejects_a_redirect_without_a_location() {
    let counter = Counter;
    let workspace = Workspace::new();
    let runtime = runtime(
        FakeTransport::new(302, "moved", &[]),
        TestCredentials::working(),
        &counter,
    );
    let error = chat::chat(&runtime, &workspace.chat(false))
        .await
        .expect_err("redirect status");
    assert_eq!((error.status_code, error.message.as_str()), (302, "moved"));
    assert!(error.headers.is_some());
}

#[tokio::test]
async fn completion_stream_skips_a_failed_frame_and_keeps_reading() {
    let counter = Counter;
    let mut workspace = Workspace::new();
    workspace.options = Options::from([(String::from("stream"), json!(true))]);
    let frames = interrupted_frames(
        br#"{"token": {"text": "first"}}"#,
        br#"{"token": {"text": "third"}}"#,
    );
    let chunks: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
    let runtime = runtime(
        FakeTransport::new(200, "", &chunks),
        TestCredentials::working(),
        &counter,
    );
    let mut events = completion::completion_stream(&runtime, &workspace.completion())
        .await
        .expect("completion stream");
    assert!(!events.state.is_messages_api);
    assert!(events.state.model.is_empty());
    let mut texts = Vec::new();
    while let Some(item) = events.next().await {
        let chunk = item.expect("the failed frame is skipped").expect("a chunk");
        let SagemakerChunk::Token(token) = chunk else {
            panic!("a token chunk is expected");
        };
        texts.push(token.text);
    }
    assert_eq!(texts, ["first", "third"]);
}

#[tokio::test]
async fn chat_stream_skips_a_failed_frame_and_keeps_reading() {
    let counter = Counter;
    let workspace = Workspace::new();
    let frames = interrupted_frames(
        br#"{"id": "1", "created": 2, "choices": [{"index": 0, "delta": {"content": "first"}}]}"#,
        br#"{"id": "1", "created": 2, "choices": [{"index": 0, "delta": {"content": "third"}}]}"#,
    );
    let chunks: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
    let runtime = runtime(
        FakeTransport::new(200, "", &chunks),
        TestCredentials::working(),
        &counter,
    );
    let mut events = chat::chat_stream(&runtime, &workspace.chat(true))
        .await
        .expect("chat stream");
    assert!(events.state.is_messages_api);
    assert!(events.state.model.is_empty());
    let mut texts = Vec::new();
    while let Some(item) = events.next().await {
        let chunk = item.expect("the failed frame is skipped").expect("a chunk");
        let SagemakerChunk::Chat(chat) = chunk else {
            panic!("a chat chunk is expected");
        };
        let content = chat.choices[0].delta.content.clone().unwrap_or_default();
        texts.push(content);
    }
    assert_eq!(texts, ["first", "third"]);
}

#[tokio::test]
async fn completion_reply_uses_the_injected_identifier() {
    let counter = Counter;
    let workspace = Workspace::new();
    let reply = completion::completion(
        &runtime(
            FakeTransport::new(200, r#"{"generated_text":"hello"}"#, &[]),
            TestCredentials::working(),
            &counter,
        ),
        &workspace.completion(),
    )
    .await
    .expect("completion reply");
    assert_eq!(
        reply.id,
        format!("chatcmpl-{}", Uuid::from_u128(IDENTIFIER))
    );
    assert_eq!(reply.created, 0);
    assert_eq!(reply.model, "sagemaker/endpoint");
    assert_eq!(reply.choices[0].message.content.as_deref(), Some("hello"));
}

#[tokio::test]
async fn missing_credentials_become_a_400_on_completion_and_chat() {
    let counter = Counter;
    let workspace = Workspace::new();
    let completion = upstream(
        completion::completion(
            &runtime(
                FakeTransport::new(200, "{}", &[]),
                TestCredentials::failing(CredentialError::Source(SourceError::Provider(
                    CredentialsError::not_loaded_no_source(),
                ))),
                &counter,
            ),
            &workspace.completion(),
        )
        .await
        .expect_err("completion credentials"),
    );
    assert_eq!(
        (completion.status_code, completion.message.as_str()),
        (400, "Unable to locate credentials")
    );
    let chat = chat::chat(
        &runtime(
            FakeTransport::new(200, "{}", &[]),
            TestCredentials::failing(CredentialError::Source(SourceError::Provider(
                CredentialsError::not_loaded_no_source(),
            ))),
            &counter,
        ),
        &workspace.chat(false),
    )
    .await
    .expect_err("chat credentials");
    assert_eq!(
        (chat.status_code, chat.message.as_str()),
        (400, "Unable to locate credentials")
    );
}

mod embedding_replies;
mod errors;
mod timeouts;

fn interrupted_frames(first: &[u8], last: &[u8]) -> [Vec<u8>; 3] {
    let payload_headers = [
        (":message-type", text_header("event")),
        (":event-type", text_header("PayloadPart")),
    ];
    [
        frame(&payload_headers, first),
        frame(
            &[
                (":message-type", text_header("exception")),
                (":exception-type", text_header("ModelStreamError")),
            ],
            br#"{"message":"boom"}"#,
        ),
        frame(&payload_headers, last),
    ]
}

fn assert_transmitted_signature(sent: &OutboundRequest) {
    let RequestBody::Json(body) = &sent.body else {
        panic!("a JSON body is expected");
    };
    let headers: Vec<_> = sent
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                Some(value.to_str().expect("a text header").to_owned()),
            )
        })
        .collect();
    let credentials = credentials();
    let signed = sign_request(&Signable {
        method: "POST",
        url: sent.url.as_str(),
        headers: &without_computed(filter_for_signature(&headers), false),
        body,
        service: "sagemaker",
        region: "us-west-2",
        credentials: &credentials,
        mode: SigningMode::SigV4Auth,
        now: UNIX_EPOCH,
    })
    .expect("the transmitted request signs");
    let computed = signed
        .iter()
        .find_map(|(name, value)| (name == "Authorization").then_some(value.as_str()))
        .expect("a computed signature");
    let transmitted = sent
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .expect("a transmitted signature");
    assert_eq!(transmitted, computed);
}
