use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_anthropic::chat_call::{ChatClients, ChatDeps, ChatOutcome};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_foundry::anthropic::request::Style;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object as PromptObject;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::runtime::Builder;

use crate::foundry_dispatch::chat::{self, Answer};
use crate::foundry_dispatch::claude;
use crate::foundry_dispatch::credentials::{Globals, no_credentials};
use crate::foundry_dispatch::fixtures::claude::{Harness, Mapper, NOW, TIMEOUT, Tally, Wire};
use crate::foundry_dispatch::fixtures::logger::RecordingLogger;
use crate::operation::OperationError;
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedHttp, PreparedRequest};

pub(super) const KEY: &str = "sk-foundry-fixture";
pub(super) const BASE: &str = "https://res.services.ai.azure.com/models";
pub(super) const HOST: &str = "https://res.services.ai.azure.com";
pub(super) const REPLY: &str = r#"{"id":"chatcmpl-foundry","object":"chat.completion","created":1700000000,"model":"provider-model","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
pub(super) const ANTHROPIC_REPLY: &str = r#"{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":2}}"#;

pub(super) fn field<'value>(value: &'value JsonValue, name: &str) -> &'value JsonValue {
    let JsonValue::Object(fields) = value else {
        panic!("the value is a JSON object");
    };
    fields.get(name).expect("the field is present")
}

pub(super) fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the runtime builds")
        .block_on(future)
}

pub(super) fn supplied(status: u16, body: &str) -> Answer<'static> {
    Answer::Supplied {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn request(body: &str) -> ChatRequest {
    serde_json::from_str(body).expect("a chat request")
}

pub(super) fn simple_request(model: &str) -> ChatRequest {
    request(&format!(
        r#"{{"model":"{model}","messages":[{{"role":"user","content":"hi"}}]}}"#
    ))
}

pub(super) fn deployment(api_base: &str, api_key: Option<&str>) -> Deployment {
    let mut params: IndexMap<String, JsonValue> = IndexMap::new();
    params.insert(
        String::from("model"),
        JsonValue::Text(String::from("deployment")),
    );
    params.insert(
        String::from("api_base"),
        JsonValue::Text(api_base.to_owned()),
    );
    if let Some(key) = api_key {
        params.insert(String::from("api_key"), JsonValue::Text(key.to_owned()));
    }
    Deployment::from_params(params).expect("the deployment")
}

pub(super) fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the policy"),
    );
    ProxiedClient::outbound(&policy).expect("the client")
}

pub(super) async fn loopback() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    (listener, format!("http://{address}/models"))
}

// Reads one request body and answers it, returning the raw request text
pub(super) async fn serve_once(listener: TcpListener, body: &[u8], reply: &str) -> String {
    let (mut stream, _peer) = listener.accept().await.expect("the connection");
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !received.ends_with(body) {
        let read = stream.read(&mut chunk).await.expect("the request bytes");
        assert_ne!(read, 0, "the client closed before sending the body");
        received.extend_from_slice(chunk.get(..read).expect("the bytes read"));
    }
    stream.write_all(reply.as_bytes()).await.expect("the reply");
    String::from_utf8(received).expect("the request text")
}

pub(super) struct Env {
    pub(super) global_api_base: Option<String>,
    pub(super) global_api_key: Option<SecretString>,
    pub(super) environment_api_base: Option<String>,
    pub(super) environment_api_key: Option<SecretString>,
    pub(super) headers: Option<Headers>,
    pub(super) credentials: DeploymentSettings,
    pub(super) environment: BTreeMap<String, String>,
}

impl Default for Env {
    fn default() -> Self {
        Self::new()
    }
}

impl Env {
    #[must_use]
    pub(super) fn new() -> Self {
        Self {
            global_api_base: None,
            global_api_key: None,
            environment_api_base: None,
            environment_api_key: None,
            headers: None,
            credentials: no_credentials(),
            environment: BTreeMap::new(),
        }
    }

    #[must_use]
    pub(super) fn global_base(self, api_base: &str) -> Self {
        Self {
            global_api_base: Some(api_base.to_owned()),
            ..self
        }
    }

    #[must_use]
    pub(super) fn global_key(self, api_key: &str) -> Self {
        Self {
            global_api_key: Some(SecretString::from(api_key)),
            ..self
        }
    }

    #[must_use]
    pub(super) fn environment_base(self, api_base: &str) -> Self {
        Self {
            environment_api_base: Some(api_base.to_owned()),
            ..self
        }
    }

    #[must_use]
    pub(super) fn environment_key(self, api_key: &str) -> Self {
        Self {
            environment_api_key: Some(SecretString::from(api_key)),
            ..self
        }
    }

    #[must_use]
    pub(super) fn headers(self, headers: Headers) -> Self {
        Self {
            headers: Some(headers),
            ..self
        }
    }

    pub(super) fn with_globals(api_base: &str, api_key: &str) -> Self {
        Self::new().global_base(api_base).global_key(api_key)
    }

    pub(super) fn with_header(name: &str, value: &str) -> Self {
        Self::new().headers(Headers::from([(name.to_owned(), value.to_owned())]))
    }

    pub(super) fn globals(&self) -> Globals<'_> {
        Globals {
            api_base: self.global_api_base.as_deref(),
            api_key: self.global_api_key.as_ref(),
            environment_api_base: self.environment_api_base.as_deref(),
            environment_api_key: self.environment_api_key.as_ref(),
            headers: self.headers.as_ref(),
            environment: &self.environment,
            credentials: &self.credentials,
            refresh_enabled: false,
            sources: &NoSources,
            token_provider: None,
        }
    }
}

pub(super) const fn chat_call<'data>(
    request: &'data ChatRequest,
    deployment: &'data Deployment,
    model: &'data str,
    stream: bool,
) -> chat::Call<'data> {
    chat::Call {
        model,
        request,
        extra_headers: None,
        headers: None,
        api_base: Some(deployment.api_base.as_str()),
        api_key: deployment.api_key.as_ref(),
        deployment,
        stream,
    }
}

pub(super) const fn chat_http(prepared: &PreparedRequest) -> &PreparedHttp {
    let PreparedRequest::Chat(chat) = prepared else {
        panic!("the call prepares a chat request");
    };
    &chat.http
}

pub(super) const fn stream_http(prepared: &PreparedRequest) -> &PreparedHttp {
    let PreparedRequest::Stream(chat) = prepared else {
        panic!("the call prepares a stream request");
    };
    &chat.http
}

pub(super) fn text_header<'headers>(
    headers: &'headers HeaderMap,
    name: &str,
) -> Option<&'headers str> {
    let value = headers.get(name)?;
    value.to_str().ok()
}

// The content of the first message in a sent body
pub(super) fn first_content(body: &JsonValue) -> &JsonValue {
    let JsonValue::Array(messages) = field(body, "messages") else {
        panic!("the sent messages are an array");
    };
    field(messages.first().expect("a message"), "content")
}

pub(super) fn run_chat(
    call: chat::Call<'_>,
    env: &Env,
    answer: Answer<'_>,
) -> (Result<chat::Outcome, OperationError>, RecordingLogger) {
    let mut logger = RecordingLogger::default();
    let result = block_on(
        chat::Dispatch {
            call,
            globals: env.globals(),
            answer,
        }
        .run(&mut logger),
    );
    (result, logger)
}

const USER_MESSAGE: &str = r#"{"role":"user","content":"hi"}"#;

// The message, parameters and key every Claude call in the cases shares
pub(super) struct ClaudeInputs {
    messages: Vec<PromptObject>,
    params: PromptObject,
    key: Option<SecretString>,
}

impl ClaudeInputs {
    pub(super) fn new() -> Self {
        Self {
            messages: vec![serde_json::from_str(USER_MESSAGE).expect("a message")],
            params: PromptObject::default(),
            key: Some(SecretString::from(KEY)),
        }
    }

    pub(super) fn call(&self, style: Style, stream: bool) -> claude::Call<'_> {
        let mut optional_params = IndexMap::from([(
            String::from("temperature"),
            Json::Text(String::from("warm")),
        )]);
        if stream {
            optional_params.insert(String::from("stream"), Json::Bool(true));
        }
        claude::Call {
            model: "claude-sonnet-4-5",
            messages: &self.messages,
            optional_params,
            litellm_params: &self.params,
            caller_headers: IndexMap::new(),
            api_base: Some(HOST),
            api_key: self.key.as_ref(),
            timeout: TIMEOUT,
            style,
            stream,
            now: NOW,
            disable_add_prefix_to_prompt: false,
        }
    }
}

// What a Claude dispatch answered, without the transport's own types
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Ran {
    Reply {
        model: Option<String>,
        content: Option<String>,
    },
    Stream,
    Embedding,
    Failed,
}

impl Ran {
    fn from_result<Failure, Counter, Problem>(
        result: Result<ChatOutcome<Failure, Counter>, Problem>,
    ) -> Self {
        match result {
            Ok(ChatOutcome::Reply(envelope)) => Self::Reply {
                model: envelope.model,
                content: envelope
                    .choices
                    .into_iter()
                    .next()
                    .and_then(|choice| choice.message.content),
            },
            Ok(ChatOutcome::Stream(_)) => Self::Stream,
            Ok(ChatOutcome::Embedding) => Self::Embedding,
            Err(_) => Self::Failed,
        }
    }
}

pub(super) fn run_claude(call: claude::Call<'_>, wire: &Wire) -> (Ran, RecordingLogger) {
    let harness = Harness::new().expect("the harness");
    let mut state = harness.state().expect("the logging state");
    let env_values = harness.env();
    let env = Env::default();
    let mut logger = RecordingLogger::default();
    let result = block_on(
        claude::Dispatch {
            call,
            globals: env.globals(),
            deps: ChatDeps {
                config: &Mapper,
                clients: ChatClients {
                    supplied: None,
                    pooled: wire,
                },
                state: &mut state,
                env: &env_values,
                stream_logger: harness.quiet(),
                counter: Tally,
                ids: harness.quiet(),
                max_event_bytes: 0x0010_0000,
                max_error_body_bytes: 0x0010_0000,
            },
        }
        .run(&mut logger),
    );
    (Ran::from_result(result), logger)
}

// The first request the wire saw, read while the lock is held
pub(super) struct Posted {
    pub(super) url: String,
    pub(super) api_key: Option<String>,
    pub(super) body: JsonValue,
}

pub(super) fn first_post(wire: &Wire) -> Posted {
    let posts = wire.recorded();
    let first = posts.first().expect("the wire saw a request");
    let url = first.url.as_str().to_owned();
    let api_key = text_header(&first.headers, "api-key").map(str::to_owned);
    let body = first.body.clone();
    drop(posts);
    Posted {
        url,
        api_key,
        body: serde_json::from_slice(&body).expect("the request body"),
    }
}
