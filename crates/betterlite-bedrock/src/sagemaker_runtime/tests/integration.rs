use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use aws_credential_types::provider::error::CredentialsError;
use aws_smithy_eventstream::frame::write_message_to;
use aws_smithy_types::event_stream::{Header, HeaderValue, Message};
use aws_smithy_types::str_bytes::StrBytes;
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
use betterlite_openai::sagemaker_completion::request::CustomPromptEntry;
use betterlite_openai::sagemaker_completion::response::TextTokenCounter;
use betterlite_prompts::arguments::json::Object as Body;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::Hub;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use secrecy::ExposeSecret;
use secrecy::SecretString;
use serde_json::Value;
use uuid::Uuid;

use crate::aws_settings::sources::{Credentials, SourceError};
use crate::credentials::error::CredentialError;
use crate::sagemaker_runtime::chat::{ChatCall, ChatVariant};
use crate::sagemaker_runtime::completion::CompletionCall;
use crate::sagemaker_runtime::embedding::{EmbeddingCall, EmbeddingReply};
use crate::sagemaker_runtime::endpoint::ParamMap;
use crate::sagemaker_runtime::runtime::{
    ChunkStream, SagemakerCredentials, SagemakerEnvironment, SagemakerRuntime, SagemakerTransport,
};
use crate::sagemaker_stream::SagemakerChunk;

const IDENTIFIER: u128 = 7;

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

struct RejectingCredentials {
    failure: Mutex<Option<CredentialError>>,
}

impl RejectingCredentials {
    fn new(failure: CredentialError) -> Self {
        Self {
            failure: Mutex::new(Some(failure)),
        }
    }
}

impl SagemakerCredentials for RejectingCredentials {
    async fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> Result<Credentials, CredentialError> {
        let failure = self.failure.lock().expect("failure lock").take();
        Err(failure.expect("the runtime resolves credentials once"))
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
        if let Some(error) = self.failure.lock().expect("failure lock").take() {
            return Err(error);
        }
        self.requests.lock().expect("request lock").push(request);
        Ok((self.status, self.chunks.clone()))
    }
}

impl SagemakerTransport for FakeTransport {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        let (status, _chunks) = self.reply(request)?;
        Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
        })
    }

    async fn open_stream(
        &self,
        request: OutboundRequest,
    ) -> Result<(u16, ChunkStream), TransportError> {
        let (status, chunks) = self.reply(request)?;
        let items = chunks
            .into_iter()
            .map(Ok::<Vec<u8>, HttpError>)
            .collect::<Vec<_>>();
        Ok((status, Box::pin(stream::iter(items))))
    }
}

fn fixed_identifier() -> Uuid {
    Uuid::from_u128(IDENTIFIER)
}

fn runtime<'counter, T, C>(
    transport: T,
    credentials: C,
    counter: &'counter Counter,
) -> SagemakerRuntime<'counter, T, C>
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
        clock: aws_smithy_async::time::SharedTimeSource::new(
            aws_smithy_async::time::StaticTimeSource::new(UNIX_EPOCH),
        ),
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
    let message = Message::new_from_parts(parts, payload.to_vec());
    let mut out = Vec::new();
    write_message_to(&message, &mut out).expect("frame");
    out
}

fn event(payload: &[u8]) -> Vec<u8> {
    frame(
        &[
            (":message-type", text_header("event")),
            (":event-type", text_header("PayloadPart")),
        ],
        payload,
    )
}

fn exception(payload: &[u8]) -> Vec<u8> {
    frame(
        &[
            (":message-type", text_header("exception")),
            (":exception-type", text_header("ModelStreamError")),
        ],
        payload,
    )
}

#[tokio::test]
async fn credentials_are_resolved_for_the_selected_region() {
    let counter = Counter;
    let runtime = runtime(
        FakeTransport::new(200, "{}", &[]),
        FakeCredentials,
        &counter,
    );
    let credentials = runtime
        .credentials_for(&ParamMap::new(), "eu-west-1")
        .await
        .expect("test credentials");
    assert_eq!(credentials.access_key_id.expose_secret(), "access");
    assert_eq!(credentials.secret_access_key.expose_secret(), "secret");
}

#[tokio::test]
async fn prepare_calls_build_sigv4_signed_completion_chat_and_embedding_requests() {
    let counter = Counter;
    let mut workspace = Workspace::new();
    workspace.options = Options::from([(String::from("stream"), Value::Bool(true))]);
    workspace.chat_options =
        serde_json::from_str(r#"{"model_id":"component"}"#).expect("chat options");
    let runtime = runtime(
        FakeTransport::new(200, "{}", &[]),
        FakeCredentials,
        &counter,
    );
    let prepared = runtime
        .prepare_completion(&workspace.completion())
        .await
        .expect("completion request");
    assert!(prepared.url.ends_with("/invocations-response-stream"));
    assert!(signed_with_sigv4(&prepared.headers));
    assert!(prepared.body_json.contains_key("inputs"));

    let prepared = runtime
        .prepare_chat(&workspace.chat(false))
        .await
        .expect("chat request");
    assert!(prepared.headers.iter().any(|(name, value)| {
        name == "X-Amzn-SageMaker-Inference-Component" && value == "component"
    }));
    assert!(signed_with_sigv4(&prepared.headers));

    let prepared = runtime
        .prepare_embedding(&workspace.embedding("voyage-3"))
        .await
        .expect("embedding request");
    assert!(prepared.call.url.ends_with("/invocations"));
    assert!(signed_with_sigv4(&prepared.call.headers));
}

#[tokio::test]
async fn completion_and_embedding_take_aws_region_before_aws_region_name() {
    let counter = Counter;
    let workspace = Workspace::new();
    let mut runtime = runtime(
        FakeTransport::new(200, "{}", &[]),
        FakeCredentials,
        &counter,
    );
    runtime.environment = SagemakerEnvironment {
        region_name: Some(String::from("eu-west-1")),
        region: Some(String::from("us-east-1")),
        session_region: Some(String::from("ap-south-1")),
    };
    let completion = runtime
        .prepare_completion(&workspace.completion())
        .await
        .expect("completion request");
    assert_eq!(completion.region, "us-east-1");
    let embedding = runtime
        .prepare_embedding(&workspace.embedding("voyage-3"))
        .await
        .expect("embedding request");
    assert_eq!(embedding.call.region, "us-east-1");
}

#[tokio::test]
async fn chat_region_comes_from_the_model_arn_before_the_environment() {
    let counter = Counter;
    let workspace = Workspace::new();
    let runtime = runtime(
        FakeTransport::new(200, "{}", &[]),
        FakeCredentials,
        &counter,
    );
    let mut chat = workspace.chat(false);
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
async fn sagemaker_nova_chat_drops_the_model_field() {
    let counter = Counter;
    let workspace = Workspace::new();
    let runtime = runtime(
        FakeTransport::new(200, "{}", &[]),
        FakeCredentials,
        &counter,
    );
    let mut chat = workspace.chat(false);
    chat.variant = ChatVariant::Chat;
    let chat_prepared = runtime.prepare_chat(&chat).await.expect("chat request");
    assert!(chat_prepared.body_json.contains_key("model"));
    let mut chat = workspace.chat(false);
    chat.variant = ChatVariant::Nova;
    let nova = runtime.prepare_chat(&chat).await.expect("nova request");
    assert!(!nova.body_json.contains_key("model"));
    assert_eq!(
        nova.body_json.get("messages"),
        chat_prepared.body_json.get("messages")
    );
    assert!(signed_with_sigv4(&nova.headers));
}

#[tokio::test]
async fn completion_component_header_is_added_after_signing() {
    let counter = Counter;
    let mut workspace = Workspace::new();
    workspace.options = Options::from([(
        String::from("model_id"),
        Value::String(String::from("component")),
    )]);
    let runtime = runtime(
        FakeTransport::new(200, "{}", &[]),
        FakeCredentials,
        &counter,
    );
    let prepared = runtime
        .prepare_completion(&workspace.completion())
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
    let plain = runtime
        .prepare_completion(&plain_workspace.completion())
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
async fn chat_reads_a_compatible_reply() {
    let counter = Counter;
    let workspace = Workspace::new();
    let reply = runtime(
        FakeTransport::new(
            200,
            r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"hi"}}]}"#,
            &[],
        ),
        FakeCredentials,
        &counter,
    )
    .chat(&workspace.chat(false))
    .await
    .expect("chat reply");
    assert_eq!(
        reply.response.choices[0].message.content.as_deref(),
        Some("hi")
    );
}

#[tokio::test]
async fn embedding_sends_the_custom_attributes_header() {
    let counter = Counter;
    let workspace = Workspace::new();
    let prepared = runtime(
        FakeTransport::new(200, "{}", &[]),
        FakeCredentials,
        &counter,
    )
    .prepare_embedding(&workspace.embedding("voyage-3"))
    .await
    .expect("embedding request");
    assert!(prepared.call.headers.iter().any(|(name, value)| {
        name == "X-Amzn-SageMaker-Custom-Attributes" && value == "accept_eula=true"
    }));
    assert!(
        prepared
            .call
            .headers
            .iter()
            .any(|(name, value)| { name == "Content-Type" && value == "application/json" })
    );
}

#[tokio::test]
async fn chat_rejects_a_redirect_without_a_location() {
    let counter = Counter;
    let workspace = Workspace::new();
    let runtime = runtime(
        FakeTransport::new(302, "moved", &[]),
        FakeCredentials,
        &counter,
    );
    let error = runtime
        .chat(&workspace.chat(false))
        .await
        .expect_err("redirect status");
    assert_eq!((error.status_code, error.message.as_str()), (302, "moved"));
    assert!(error.headers.is_some());
}

#[tokio::test]
async fn runtime_calls_preserve_error_status_and_stream_policies() {
    let counter = Counter;
    let mut workspace = Workspace::new();
    let completion = runtime(
        FakeTransport::new(429, "throttled", &[]),
        FakeCredentials,
        &counter,
    )
    .completion(&workspace.completion())
    .await
    .expect_err("completion status");
    assert_eq!(
        (completion.status_code, completion.message.as_str()),
        (429, "throttled")
    );

    let chat = runtime(
        FakeTransport::new(429, "chat throttled", &[]),
        FakeCredentials,
        &counter,
    )
    .chat(&workspace.chat(false))
    .await
    .expect_err("chat status");
    assert_eq!(
        (chat.status_code, chat.message.as_str()),
        (429, "chat throttled")
    );

    workspace.options = Options::from([(String::from("stream"), Value::Bool(true))]);
    let completion_stream = runtime(
        FakeTransport::new(201, "", &[b"failed"]),
        FakeCredentials,
        &counter,
    )
    .completion_stream(&workspace.completion())
    .await;
    assert!(
        matches!(completion_stream, Err(error) if error.status_code == 201 && error.message == "failed")
    );

    let chat_stream = runtime(
        FakeTransport::new(201, "", &[b"failed"]),
        FakeCredentials,
        &counter,
    )
    .chat_stream(&workspace.chat(true))
    .await;
    assert!(
        matches!(chat_stream, Err(error) if error.status_code == 201 && error.message == "failed")
    );

    let embedding = runtime(
        FakeTransport::new(503, "embedding failed", &[]),
        FakeCredentials,
        &counter,
    )
    .embedding(&workspace.embedding("voyage-3"))
    .await
    .expect_err("embedding status");
    assert_eq!(
        (embedding.status_code, embedding.message.as_str()),
        (503, "embedding failed")
    );
}

#[tokio::test]
async fn completion_stream_skips_a_failed_frame_and_keeps_reading() {
    let counter = Counter;
    let mut workspace = Workspace::new();
    workspace.options = Options::from([(String::from("stream"), Value::Bool(true))]);
    let frames = [
        event(br#"{"token": {"text": "first"}}"#),
        exception(br#"{"message": "boom"}"#),
        event(br#"{"token": {"text": "third"}}"#),
    ];
    let chunks: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
    let runtime = runtime(
        FakeTransport::new(200, "", &chunks),
        FakeCredentials,
        &counter,
    );
    let mut events = runtime
        .completion_stream(&workspace.completion())
        .await
        .expect("completion stream");
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
    let frames = [
        event(br#"{"id": "1", "created": 2, "choices": [{"index": 0, "delta": {"content": "first"}}]}"#),
        exception(br#"{"message": "boom"}"#),
        event(br#"{"id": "1", "created": 2, "choices": [{"index": 0, "delta": {"content": "third"}}]}"#),
    ];
    let chunks: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
    let runtime = runtime(
        FakeTransport::new(200, "", &chunks),
        FakeCredentials,
        &counter,
    );
    let mut events = runtime
        .chat_stream(&workspace.chat(true))
        .await
        .expect("chat stream");
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
    let reply = runtime(
        FakeTransport::new(200, r#"{"generated_text":"hello"}"#, &[]),
        FakeCredentials,
        &counter,
    )
    .completion(&workspace.completion())
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
    let completion = runtime(
        FakeTransport::new(200, "{}", &[]),
        RejectingCredentials::new(CredentialError::Source(SourceError::Provider(
            CredentialsError::not_loaded_no_source(),
        ))),
        &counter,
    )
    .completion(&workspace.completion())
    .await
    .expect_err("completion credentials");
    assert_eq!(
        (completion.status_code, completion.message.as_str()),
        (400, "Unable to locate credentials")
    );
    let chat = runtime(
        FakeTransport::new(200, "{}", &[]),
        RejectingCredentials::new(CredentialError::Source(SourceError::Provider(
            CredentialsError::not_loaded_no_source(),
        ))),
        &counter,
    )
    .chat(&workspace.chat(false))
    .await
    .expect_err("chat credentials");
    assert_eq!(
        (chat.status_code, chat.message.as_str()),
        (400, "Unable to locate credentials")
    );
}

#[tokio::test]
async fn transport_timeouts_become_a_408() {
    let counter = Counter;
    let workspace = Workspace::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let address = listener.local_addr().expect("listener address");
    let client = reqwest::ClientBuilder::new()
        .no_proxy()
        .read_timeout(Duration::from_millis(100))
        .build()
        .expect("timeout client");
    let failure = client
        .get(format!("http://{address}/invocations"))
        .send()
        .await
        .expect_err("the listener never answers");
    let timeout = runtime(
        FakeTransport::failure(TransportError::sending(failure)),
        FakeCredentials,
        &counter,
    )
    .completion(&workspace.completion())
    .await
    .expect_err("timeout status");
    assert_eq!(timeout.status_code, 408);
}

#[tokio::test]
async fn embedding_projects_a_voyage_reply() {
    let counter = Counter;
    let workspace = Workspace::new();
    let reply = runtime(
        FakeTransport::new(200, r#"{"data":[{"embedding":[1,2]}]}"#, &[]),
        FakeCredentials,
        &counter,
    )
    .embedding(&workspace.embedding("voyage-3"))
    .await
    .expect("voyage reply");
    assert!(matches!(reply, EmbeddingReply::Voyage(_)));
}

#[tokio::test]
async fn embedding_rejects_an_unreadable_successful_reply() {
    let counter = Counter;
    let workspace = Workspace::new();
    let error = runtime(
        FakeTransport::new(200, "not-json", &[]),
        FakeCredentials,
        &counter,
    )
    .embedding(&workspace.embedding("voyage-3"))
    .await
    .expect_err("unreadable reply");
    assert_eq!(error.status_code, 500);
    assert!(error.message.contains("not-json"));
}
