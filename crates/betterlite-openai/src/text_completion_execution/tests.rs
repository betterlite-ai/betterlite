#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo, ProxyTrust};
use betterlite_error::envelope::WireField;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::StreamExt as _;
use http::HeaderMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Map, Value};
use tempfile::NamedTempFile;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OnceCell, oneshot};
use tokio::time::timeout;
use tracing::Level;
use tracing::subscriber::with_default;

use super::catalog::{CompletionCatalog, CompletionRoute, route_for};
use super::failure::{CompletionFailure, wrapped_failure};
use super::http::{HttpCompletionConstructor, HttpCompletionSdk, UpstreamFacts};
use super::log::TracingCompletionLog;
use super::transport_request;
use super::{CompletionCallAnswer, CompletionExecutor, CompletionSend, answered, status_failure};
use crate::chat::deployment::{ChatDeployment, OpenAiDefaults, Provider};
use crate::text_completion_request::{CompletionPrompt, CompletionRequest};
use crate::text_completion_transport::error::{ErrorSource, ErrorStatus, TextCompletionError};
use crate::text_completion_transport::logging::{
    CompletionLogger as _, PostCallRecord, PreCallRecord, ResponseKind, SecretHeaders,
};
use crate::text_completion_transport::sdk::{
    ClientConstructor as _, HttpClientSource, RawCompletion, SdkArguments, SdkError, StatusError,
    TextCompletionSdk as _, status_error,
};
use betterlite_http::client_kind::ClientKind;

fn deployment(model: &str) -> ChatDeployment {
    let listed = ModelDeployment {
        model_name: String::from("g"),
        litellm_params: LiteLlmParams {
            ssl_verify: None,
            model: Some(model.to_owned()),
            api_base: Some(String::from("http://127.0.0.1:9/v1")),
            api_key: Some(SecretString::from("configured-key")),
            api_version: None,
            timeout: None,
            stream_timeout: None,
            max_retries: None,
            rpm: None,
            tpm: None,
            weight: None,
        },
        model_info: ModelInfo::default(),
    };
    ChatDeployment::resolve(&listed, &OpenAiDefaults::default()).expect("deployment")
}

fn catalog() -> CompletionCatalog {
    CompletionCatalog {
        chat_models: BTreeSet::from([String::from("gpt-4o")]),
        text_models: BTreeSet::from([
            String::from("gpt-3.5-turbo-instruct"),
            String::from("text-davinci-003"),
        ]),
    }
}

fn completion(prompt: &str) -> CompletionRequest {
    let body = format!(r#"{{"model":"gpt-3.5-turbo-instruct","prompt":{prompt}}}"#);
    CompletionRequest::parse(body.as_bytes()).expect("completion request")
}

// a deployment that inherited litellm_settings.organization or OPENAI_ORGANIZATION
fn deployment_with_organization(model: &str) -> ChatDeployment {
    let mut deployment = deployment(model);
    deployment.organization = Some(String::from("org-1"));
    deployment
}

// a provider whose identity resolution refused it, the way a text-completion or azure prefix resolves today
fn unsupported(model: &str) -> ChatDeployment {
    let mut deployment = deployment("openai/gpt-4o");
    model.clone_into(&mut deployment.litellm_model);
    deployment.provider = Provider::Unsupported {
        model: model.to_owned(),
    };
    deployment
}

fn provider_error(status: u16) -> TextCompletionError {
    TextCompletionError {
        status: ErrorStatus::Code(status),
        message: String::from("the provider refused the call"),
        headers: None,
        source: Box::new(ErrorSource::Raised),
    }
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    StringPrompt,
    StringsPrompt,
    TokensPrompt,
    TokenListsPrompt,
    EmptyStringPrompt,
    EmptyListRejected,
    MixedPromptRejected,
    ChatCatalogSelection,
    BundledCatalog,
    ProviderErrorFacts,
    ConfiguredDestination,
    UnsupportedProvider,
}

#[rstest]
#[case::string_prompt(Scenario::StringPrompt)]
#[case::strings_prompt(Scenario::StringsPrompt)]
#[case::tokens_prompt(Scenario::TokensPrompt)]
#[case::token_lists_prompt(Scenario::TokenListsPrompt)]
#[case::empty_string_prompt(Scenario::EmptyStringPrompt)]
#[case::empty_list_rejected(Scenario::EmptyListRejected)]
#[case::mixed_prompt_rejected(Scenario::MixedPromptRejected)]
#[case::chat_catalog_selection(Scenario::ChatCatalogSelection)]
#[case::bundled_catalog(Scenario::BundledCatalog)]
#[case::provider_error_facts(Scenario::ProviderErrorFacts)]
#[case::configured_destination(Scenario::ConfiguredDestination)]
#[case::unsupported_provider(Scenario::UnsupportedProvider)]
fn text_completions_text_completion_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::StringPrompt => string_prompt_scenario(),
        Scenario::StringsPrompt => strings_prompt_scenario(),
        Scenario::TokensPrompt => tokens_prompt_scenario(),
        Scenario::TokenListsPrompt => token_lists_prompt_scenario(),
        Scenario::EmptyStringPrompt => empty_string_prompt_scenario(),
        Scenario::EmptyListRejected => empty_list_rejected_scenario(),
        Scenario::MixedPromptRejected => mixed_prompt_rejected_scenario(),
        Scenario::ChatCatalogSelection => chat_catalog_selection_scenario(),
        Scenario::BundledCatalog => bundled_catalog_scenario(),
        Scenario::ProviderErrorFacts => provider_error_facts_scenario(),
        Scenario::ConfiguredDestination => configured_destination_scenario(),
        Scenario::UnsupportedProvider => unsupported_provider_scenario(),
    }
}

fn string_prompt_scenario() {
    let request = completion(r#""hello""#);
    let messages = request.prompt.messages().expect("messages");
    assert_eq!(messages.len(), 1);
}

fn strings_prompt_scenario() {
    let request = completion(r#"["a","b"]"#);
    let messages = request.prompt.messages().expect("messages");
    assert_eq!(messages.len(), 2);
}

fn tokens_prompt_scenario() {
    let request = completion("[1,2]");
    let messages = request.prompt.messages().expect("messages");
    assert_eq!(messages.len(), 1);
}

fn token_lists_prompt_scenario() {
    let request = completion("[[1,2],[3]]");
    let messages = request.prompt.messages().expect("messages");
    assert_eq!(messages.len(), 1);
}

fn empty_string_prompt_scenario() {
    let request = completion(r#""""#);
    assert!(request.prompt.messages().is_some());
}

fn empty_list_rejected_scenario() {
    let request = completion("[]");
    assert!(matches!(&request.prompt, CompletionPrompt::Empty));
    let failure = transport_request(&deployment("openai/gpt-3.5-turbo-instruct"), &request, "m")
        .err()
        .expect("empty prompt");
    assert_eq!(failure.status, 400);
    assert_eq!(failure.error_type.as_deref(), Some("invalid_request_error"));
}

fn mixed_prompt_rejected_scenario() {
    let request = completion(r#"["a",1]"#);
    assert!(request.prompt.messages().is_none());
}

fn chat_catalog_selection_scenario() {
    assert_eq!(
        route_for(&deployment("openai/gpt-4o"), &catalog()),
        CompletionRoute::Chat {
            model: String::from("gpt-4o")
        }
    );
    assert_eq!(
        route_for(&deployment("openai/gpt-3.5-turbo-instruct"), &catalog()),
        CompletionRoute::Text {
            model: String::from("gpt-3.5-turbo-instruct")
        }
    );
    assert_eq!(
        route_for(&unsupported("text-completion-openai/gpt-4o"), &catalog()),
        CompletionRoute::Text {
            model: String::from("gpt-4o")
        }
    );
    assert_eq!(
        route_for(&unsupported("gpt-3.5-turbo-instruct"), &catalog()),
        CompletionRoute::Text {
            model: String::from("gpt-3.5-turbo-instruct")
        }
    );
    // an unprefixed name still needs the text catalog, so a completion model newer than the bundle refuses
    assert_eq!(
        route_for(&unsupported("some-new-model"), &catalog()),
        CompletionRoute::Unsupported {
            provider: String::from("some-new-model")
        }
    );
}

fn bundled_catalog_scenario() {
    let bundled = CompletionCatalog::new(&BundledCatalog::default().get());
    assert!(bundled.chat_models.contains("gpt-4o"));
    assert!(bundled.text_models.contains("gpt-3.5-turbo-instruct"));
    assert!(!bundled.text_models.contains("gpt-4o"));
}

fn provider_error_facts_scenario() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor");
    let failure = wrapped_failure(&provider_error(429), &redactor);
    assert_eq!(failure.status, 429);
    assert_eq!(failure.error_type.as_deref(), Some("throttling_error"));
    assert_eq!(failure.param, None);
    assert!(!failure.message.is_empty());
    let refused = wrapped_failure(&provider_error(400), &redactor);
    assert_eq!(refused.status, 400);
    assert_eq!(refused.error_type, None);
}

fn configured_destination_scenario() {
    let request = completion(r#""hi""#);
    let deployment = deployment("openai/gpt-3.5-turbo-instruct");
    let built = transport_request(&deployment, &request, "gpt-3.5-turbo-instruct")
        .expect("transport request");
    assert_eq!(built.api_base.as_deref(), Some("http://127.0.0.1:9/v1"));
    assert!(built.timeout > 0.0_f64);
    assert!(built.acompletion);
}

fn status_of(error: SdkError) -> StatusError {
    let SdkError::Status(status) = error else {
        panic!("a status error");
    };
    *status
}

// the streamed async path lets the SDK's own status error through, so its class names the wire type
#[test]
fn a_raw_sdk_status_error_answers_the_class_type() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor");
    let failure = status_failure(
        &StatusError {
            status: Some(500),
            message: String::from("Error code: 500 - "),
            headers: HeaderMap::new(),
            error_type: None,
            param: None,
            code: None,
        },
        &redactor,
    );
    assert_eq!(failure.status, 500);
    assert_eq!(failure.error_type.as_deref(), Some("internal_server_error"));
    assert_eq!(failure.param, None);
}

// _map_openai_exception forwards the SDK error's body to 400, 422, 429 and 500, so the route answers its type and param
#[test]
fn a_streamed_status_error_answers_the_provider_type_and_param() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor");
    let prompt_error = status_of(status_error(
        400,
        HeaderMap::new(),
        br#"{"error":{"type":"invalid_request_error","param":"prompt","code":"invalid"}}"#,
    ));
    let refused = status_failure(&prompt_error, &redactor);
    assert_eq!(refused.status, 400);
    assert_eq!(refused.error_type.as_deref(), Some("invalid_request_error"));
    assert_eq!(refused.param.as_deref(), Some("prompt"));

    // InternalServerError and RateLimitError overwrite self.type with their literal after openai.APIError reads the body
    let server_error = status_of(status_error(
        500,
        HeaderMap::new(),
        br#"{"error":{"type":"server_error","param":"model"}}"#,
    ));
    let internal = status_failure(&server_error, &redactor);
    assert_eq!(internal.status, 500);
    assert_eq!(
        internal.error_type.as_deref(),
        Some("internal_server_error")
    );
    assert_eq!(internal.param.as_deref(), Some("model"));

    let rate_error = status_of(status_error(
        429,
        HeaderMap::new(),
        br#"{"error":{"type":"requests","param":"messages"}}"#,
    ));
    let limited = status_failure(&rate_error, &redactor);
    assert_eq!(limited.status, 429);
    assert_eq!(limited.error_type.as_deref(), Some("throttling_error"));
    assert_eq!(limited.param.as_deref(), Some("messages"));
}

// getattr(e, field, "None") keeps a null field null, hands a missing one the literal "None" and passes text through
#[test]
fn a_missing_exception_field_answers_the_literal_none() {
    let answered_text = answered(WireField::Text(String::from("prompt")));
    assert_eq!(answered_text.as_deref(), Some("prompt"));
    assert_eq!(answered(WireField::Null), None);
    assert_eq!(answered(WireField::MISSING).as_deref(), Some("None"));
}

fn unsupported_provider_scenario() {
    assert!(matches!(
        route_for(&unsupported("azure/gpt-35-turbo"), &catalog()),
        CompletionRoute::Unsupported { .. }
    ));
}

#[test]
fn an_unmapped_prompt_fails_before_any_provider_call() {
    let request = completion("[]");
    let empty = transport_request(&deployment("openai/gpt-3.5-turbo-instruct"), &request, "m")
        .err()
        .expect("empty prompt");
    assert_eq!(empty.status, 400);
    assert_eq!(empty.error_type.as_deref(), Some("invalid_request_error"));
    assert!(empty.message.contains("prompt"), "{}", empty.message);

    let unmapped = completion(r#"["a",1]"#);
    let unmapped_failure =
        transport_request(&deployment("openai/gpt-3.5-turbo-instruct"), &unmapped, "m")
            .err()
            .expect("unmapped prompt");
    assert_eq!(unmapped_failure.status, 500);
    assert_eq!(unmapped_failure.error_type, None);

    let unreadable = completion(r#"[["a"]]"#);
    let unreadable_failure = transport_request(
        &deployment("openai/gpt-3.5-turbo-instruct"),
        &unreadable,
        "m",
    )
    .err()
    .expect("unreadable prompt");
    assert_eq!(unreadable_failure.status, 500);
    assert_eq!(
        unreadable_failure.error_type.as_deref(),
        Some("internal_server_error")
    );
}

#[test]
fn the_completion_call_takes_the_body_timeout_and_never_the_configured_organization() {
    let deployment = deployment_with_organization("openai/gpt-3.5-turbo-instruct");
    let request = completion(r#""hi""#);
    let built = transport_request(&deployment, &request, "gpt-3.5-turbo-instruct")
        .expect("transport request");
    assert!(built.organization.is_none());
    assert!((built.timeout - deployment.timeout_seconds).abs() < f64::EPSILON);

    let timed = CompletionRequest::parse(
        br#"{"model":"gpt-3.5-turbo-instruct","prompt":"hi","timeout":2.5}"#,
    )
    .expect("completion request");
    let timed_built = transport_request(&deployment, &timed, "gpt-3.5-turbo-instruct")
        .expect("transport request");
    assert!((timed_built.timeout - 2.5).abs() < f64::EPSILON);
}

#[test]
fn the_route_logger_records_counts_and_never_the_prompt() {
    let log = NamedTempFile::new().expect("log file");
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(Level::DEBUG)
        .with_writer(Arc::new(log.reopen().expect("log handle")))
        .finish();
    let defaults = with_default(subscriber, || {
        let mut logger = TracingCompletionLog;
        record_call(&mut logger);
        logger.stream_defaults()
    });
    let text = fs::read_to_string(log.path()).expect("log text");
    assert!(text.contains("text completion pre-call"), "{text}");
    assert!(text.contains("messages=1"), "{text}");
    assert!(text.contains("http://upstream.test/v1"), "{text}");
    assert!(text.contains("text completion post-call"), "{text}");
    assert!(text.contains("SdkObject"), "{text}");
    assert!(!text.contains("secret prompt"), "{text}");
    assert!(defaults.stream_options.is_none());
    assert!(!defaults.merge_reasoning_in_choices);
    assert!(!defaults.is_function_call);
    assert!(defaults.base_hidden_params.is_empty());
}

fn record_call(logger: &mut TracingCompletionLog) {
    logger
        .pre_call(PreCallRecord {
            input: vec![Value::Object(Map::from_iter([
                (String::from("role"), Value::from("user")),
                (String::from("content"), Value::from("secret prompt")),
            ]))],
            api_key: None,
            headers: SecretHeaders::default(),
            api_base: Some(String::from("http://upstream.test/v1")),
            complete_input_dict: Map::new(),
        })
        .expect("pre-call");
    logger.post_call(PostCallRecord {
        api_key: None,
        original_response: Value::Null,
        kind: ResponseKind::SdkObject,
        headers: SecretHeaders::default(),
        api_base: None,
    });
}

// the request head the completion client sent, read byte by byte so no header line is cut short
async fn read_completion_head(stream: &mut TcpStream) -> String {
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        let Ok(byte) = stream.read_u8().await else {
            break;
        };
        received.push(byte);
    }
    String::from_utf8(received).unwrap_or_default()
}

// a provider that takes the whole request before it answers, so a buffered client never sees a reset while writing
async fn serve(listener: &TcpListener, response: &str) -> String {
    let Ok(Ok((mut stream, _))) = timeout(Duration::from_secs(5), listener.accept()).await else {
        return String::new();
    };
    let head = read_completion_head(&mut stream).await;
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or_default();
    let mut body = vec![0_u8; length];
    let _filled = stream.read_exact(&mut body).await;
    let _written = stream.write_all(response.as_bytes()).await;
    head
}

// the provider's buffered answer, which the SDK decodes into its Completion model before the executor reads it
fn text_completion_response() -> String {
    let answer = r#"{"id":"cmpl-1","object":"text_completion","created":1700000000,"model":"upstream","choices":[{"text":"Hello","index":0,"logprobs":null,"finish_reason":"stop"}]}"#;
    format!(
        "HTTP/1.1 200 X\r\ncontent-length: {}\r\nx-upstream: kept\r\nconnection: close\r\n\r\n{answer}",
        answer.len()
    )
}

async fn serve_text_completion(listener: TcpListener) -> String {
    serve(&listener, &text_completion_response()).await
}

const REFUSED_PROMPT: &str = r#"{"error":{"message":"invalid prompt","type":"invalid_request_error","param":"prompt","code":"invalid"}}"#;

const REFUSED_SERVER: &str = r#"{"error":{"message":"the server failed","type":"server_error","param":"model","code":null}}"#;

// the error object a refusing provider answers with
fn refusal_response(status: u16, answer: &str) -> String {
    format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
        answer.len()
    )
}

// a provider that refuses the request with the error object it answers
async fn serve_refusal(listener: &TcpListener, status: u16, answer: &str) -> String {
    serve(listener, &refusal_response(status, answer)).await
}

// the async SDK a deployment builds against the fake provider at this address
fn async_sdk(address: SocketAddr, facts: Arc<Mutex<Option<UpstreamFacts>>>) -> HttpCompletionSdk {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("policy");
    let client = ProxiedClient::outbound(&Arc::new(policy)).expect("client");
    let arguments = SdkArguments {
        api_key: None,
        base_url: Some(format!("http://{address}/v1")),
        timeout: 5.0,
        max_retries: None,
        organization: None,
        http_client: HttpClientSource::AsyncHttpClient,
    };
    HttpCompletionConstructor {
        client,
        // deployment.credential() hands the client a complete header value, bearer prefix included
        authorization: Some(SecretString::from("Bearer sk-deployment")),
        facts,
    }
    .construct(&arguments, ClientKind::Asynchronous)
    .expect("sdk")
    .expect("a credential builds the sdk")
}

// the completion body an async SDK streams for the fake provider
fn streamed_completion_data() -> Map<String, Value> {
    let mut data = Map::new();
    data.insert(String::from("model"), Value::from("gpt-3.5-turbo-instruct"));
    data.insert(String::from("prompt"), Value::from("hi"));
    data.insert(String::from("stream"), Value::Bool(true));
    data
}

#[tokio::test]
async fn the_http_sdk_sends_the_credential_and_never_an_organization() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let sdk = async_sdk(address, Arc::new(Mutex::new(None)));
    assert_eq!(sdk.kind(), ClientKind::Asynchronous);
    let mut data = Map::new();
    data.insert(String::from("model"), Value::from("gpt-3.5-turbo-instruct"));
    data.insert(String::from("prompt"), Value::from("hi"));
    let (answer, head) = tokio::join!(sdk.create(&data), serve_text_completion(listener));
    let created = answer.expect("completion answer");
    assert!(matches!(created, RawCompletion::Buffered { .. }));
    let sent = head.to_ascii_lowercase();
    assert!(sent.starts_with("post /v1/completions "), "{head}");
    assert!(
        sent.contains("authorization: bearer sk-deployment"),
        "{head}"
    );
    assert!(!sent.contains("openai-organization"), "{head}");
}

// a provider that writes its first event and then waits for the client to read it before the rest
async fn serve_text_stream(listener: TcpListener, released: oneshot::Receiver<()>) {
    let Ok(Ok((mut stream, _))) = timeout(Duration::from_secs(5), listener.accept()).await else {
        return;
    };
    let head = read_completion_head(&mut stream).await;
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or_default();
    let mut body = vec![0_u8; length];
    let _filled = stream.read_exact(&mut body).await;
    let opening = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
    let first = "data: {\"id\":\"cmpl-1\",\"object\":\"text_completion\",\"choices\":[{\"text\":\" hello\",\"index\":0,\"finish_reason\":null}]}\n\n";
    let _opening_written = stream.write_all(opening.as_bytes()).await;
    let _first_written = stream.write_all(first.as_bytes()).await;
    let _first_flushed = stream.flush().await;
    let _released = timeout(Duration::from_secs(5), released).await;
    let last = "data: {\"id\":\"cmpl-1\",\"object\":\"text_completion\",\"choices\":[{\"text\":\" there\",\"index\":0,\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
    let _last_written = stream.write_all(last.as_bytes()).await;
    let _last_flushed = stream.flush().await;
}

#[tokio::test]
async fn the_http_sdk_streams_provider_events_as_they_arrive() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let facts = Arc::new(Mutex::new(None));
    let sdk = async_sdk(address, Arc::clone(&facts));
    let data = streamed_completion_data();
    let (release, released) = oneshot::channel();
    tokio::join!(
        read_streamed_completion(&sdk, &data, release),
        serve_text_stream(listener, released)
    );
    let recorded = facts.lock().expect("facts").take().expect("recorded facts");
    assert_eq!(
        recorded
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
}

// the SDK reads a refused stream's body, so the exception keeps the error object the provider sent
#[tokio::test]
async fn a_refused_stream_answers_the_provider_error_object() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let sdk = async_sdk(address, Arc::new(Mutex::new(None)));
    let data = streamed_completion_data();
    let ((refused, internal), _served) = tokio::join!(
        async {
            let refused = sdk
                .create(&data)
                .await
                .err()
                .expect("the provider refused the call");
            let internal = sdk
                .create(&data)
                .await
                .err()
                .expect("the provider refused the call");
            (refused, internal)
        },
        async {
            let first = serve_refusal(&listener, 400, REFUSED_PROMPT).await;
            let second = serve_refusal(&listener, 500, REFUSED_SERVER).await;
            (first, second)
        }
    );
    let SdkError::Status(refusal) = refused else {
        panic!("a status error");
    };
    assert_eq!(refusal.status, Some(400));
    assert_eq!(refusal.error_type.as_deref(), Some("invalid_request_error"));
    assert_eq!(refusal.param.as_deref(), Some("prompt"));
    assert!(refusal.message.contains("invalid prompt"), "{refusal:?}");
    let SdkError::Status(server) = internal else {
        panic!("a status error");
    };
    assert_eq!(server.status, Some(500));
    assert_eq!(server.error_type.as_deref(), Some("server_error"));
    assert_eq!(server.param.as_deref(), Some("model"));
}

// The client half of the streaming test: the first event arrives while the provider still waits.
async fn read_streamed_completion(
    sdk: &HttpCompletionSdk,
    data: &Map<String, Value>,
    release: oneshot::Sender<()>,
) {
    let raw = timeout(Duration::from_secs(5), sdk.create(data))
        .await
        .expect("the answer starts before the provider finishes")
        .expect("stream answer");
    let RawCompletion::Stream {
        status,
        headers,
        mut body,
    } = raw
    else {
        panic!("a stream request never buffers");
    };
    assert_eq!(status, 200);
    assert_eq!(
        headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
    let mut received = Vec::new();
    while !received.windows(2).any(|pair| pair == b"\n\n") {
        let chunk = timeout(Duration::from_secs(5), body.next())
            .await
            .expect("the first event arrives while the provider waits")
            .expect("a body chunk")
            .expect("no body error");
        received.extend_from_slice(&chunk);
    }
    let first = String::from_utf8(received).expect("utf8");
    assert!(first.contains("\"text\":\" hello\""), "{first}");
    release.send(()).expect("the provider is still streaming");
    let mut rest = Vec::new();
    while let Some(chunk) = body.next().await {
        rest.extend_from_slice(&chunk.expect("no body error"));
    }
    let rest_text = String::from_utf8(rest).expect("utf8");
    assert!(rest_text.contains("\"text\":\" there\""), "{rest_text}");
    assert!(rest_text.ends_with("data: [DONE]\n\n"), "{rest_text}");
}

// The answer shape a transport or an executor was built for.
#[derive(Clone, Copy)]
enum Shape {
    Buffered,
    Streamed,
}

// a send whose transport was built for one shape and whose executor was asked for another, answered by the given provider response
async fn crossed_send(
    transport_shape: Shape,
    send_shape: Shape,
    served: &str,
) -> Result<CompletionCallAnswer, CompletionFailure> {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("policy");
    let client = ProxiedClient::outbound(&Arc::new(policy)).expect("client");
    let executor = CompletionExecutor {
        client,
        bundled_catalog: BundledCatalog::default().get(),
        catalog: OnceCell::new(),
    };
    let mut deployment = deployment("openai/gpt-3.5-turbo-instruct");
    deployment.api_base = format!("http://{address}/v1");
    let request = completion(r#""hi""#);
    let transport_streamed = matches!(transport_shape, Shape::Streamed);
    let transport = executor
        .text_transport(&deployment, &request, transport_streamed)
        .await
        .expect("transport");
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("redactor");
    let mut logger = TracingCompletionLog;
    let (answer, _head) = tokio::join!(
        executor.send(CompletionSend {
            deployment: &deployment,
            request: transport,
            streamed: matches!(send_shape, Shape::Streamed),
            redactor: &redactor,
            logger: &mut logger,
        }),
        serve(&listener, served)
    );
    answer
}

#[tokio::test]
async fn a_buffered_completion_keeps_the_provider_headers() {
    let answer = crossed_send(
        Shape::Buffered,
        Shape::Buffered,
        &text_completion_response(),
    )
    .await
    .expect("completion answer");
    let CompletionCallAnswer::Buffered(buffered) = answer else {
        panic!("a buffered request never streams");
    };
    assert_eq!(
        buffered
            .upstream_headers
            .get("x-upstream")
            .and_then(|value| value.to_str().ok()),
        Some("kept")
    );
}

// a streamed send takes the provider's byte stream, with the headers the answer arrived with
#[tokio::test]
async fn a_streamed_send_answers_the_provider_stream() {
    let events = concat!(
        "data: {\"id\":\"cmpl-1\",\"object\":\"text_completion\",\"created\":1700000000,\"model\":\"upstream\",\"choices\":[{\"text\":\" hello\",\"index\":0,\"logprobs\":null,\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"cmpl-1\",\"object\":\"text_completion\",\"created\":1700000000,\"model\":\"upstream\",\"choices\":[{\"text\":\" there\",\"index\":0,\"logprobs\":null,\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let served = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{events}",
        events.len()
    );
    let answer = crossed_send(Shape::Streamed, Shape::Streamed, &served)
        .await
        .expect("stream answer");
    let CompletionCallAnswer::Streamed(streamed) = answer else {
        panic!("a streamed request never buffers");
    };
    assert_eq!(
        streamed
            .upstream_headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
    let mut body = streamed.stream;
    let first = timeout(Duration::from_secs(5), body.next())
        .await
        .expect("the first event arrives")
        .expect("a chunk")
        .expect("shaped chunk");
    assert_eq!(first.choices[0].delta.content.as_deref(), Some(" hello"));
}

#[tokio::test]
async fn a_send_never_takes_the_provider_shape_it_did_not_ask_for() {
    let Err(stream_sent) = crossed_send(
        Shape::Buffered,
        Shape::Streamed,
        &text_completion_response(),
    )
    .await
    else {
        panic!("a buffered answer cannot fill a stream");
    };
    assert_eq!(stream_sent.status, 500);
    assert_eq!(
        stream_sent.error_type.as_deref(),
        Some("internal_server_error")
    );
    assert!(
        stream_sent.message.contains("buffered completion"),
        "{}",
        stream_sent.message
    );
    let Err(buffer_sent) = crossed_send(
        Shape::Streamed,
        Shape::Buffered,
        &text_completion_response(),
    )
    .await
    else {
        panic!("a stream cannot fill a buffered answer");
    };
    assert_eq!(buffer_sent.status, 500);
    assert_eq!(
        buffer_sent.error_type.as_deref(),
        Some("internal_server_error")
    );
    assert!(
        buffer_sent.message.contains("byte stream"),
        "{}",
        buffer_sent.message
    );
}

// a streamed provider refusal reaches the route's failure with the provider's own type and param
#[tokio::test]
async fn a_streamed_provider_refusal_answers_its_body_through_the_executor() {
    let Err(refused) = crossed_send(
        Shape::Streamed,
        Shape::Streamed,
        &refusal_response(400, REFUSED_PROMPT),
    )
    .await
    else {
        panic!("a refused stream never answers with frames");
    };
    assert_eq!(refused.status, 400);
    assert_eq!(refused.error_type.as_deref(), Some("invalid_request_error"));
    assert_eq!(refused.param.as_deref(), Some("prompt"));
}
