use super::{
    EMBEDDING_BODY, SMALL, UNAUTHORIZED_BODY, arguments, facts, mock_answer_of, parse, payload_of,
    session, session_from, text_of,
};
use crate::completion::setup::global_credentials;
use crate::embeddings::call::{Entry, call};
use crate::embeddings::convert::{EmbeddingCallWire, EmbeddingSettingsWire};
use crate::embeddings::error::{CallFailure, Raised, operation_failure, raised};
use crate::embeddings::response::EmbeddingAnswer;
use crate::embeddings::session::EmbeddingSession;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_provider::operation::OperationError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::HeaderMap;
use pyo3_async_runtimes::tokio::get_runtime;
use rstest::rstest;
use secrecy::ExposeSecret;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::str;
use std::thread;
#[test]
fn request_a_session_needs_its_settings_text() {
    EmbeddingSession::build("not json").err().unwrap();
    let built_session = session();
    assert!(built_session.environment.is_empty());
}
#[rstest]
#[case::bad_request(Raised::BadRequest, "BadRequestError", Some(400_u16))]
#[case::internal_server(Raised::InternalServer, "InternalServerError", Some(500_u16))]
#[case::timeout(Raised::Timeout, "Timeout", Some(408_u16))]
#[case::unsupported_params(Raised::UnsupportedParams, "UnsupportedParamsError", Some(400_u16))]
#[case::plain(Raised::Plain, "Exception", None)]
fn error_raised_classes_keep_their_status(
    #[case] kind: Raised,
    #[case] name: &str,
    #[case] status: Option<u16>,
) {
    let payload = payload_of(raised(
        kind,
        String::from("boom"),
        Some(String::from(SMALL)),
        None,
    ));
    assert_eq!(payload.kind, name);
    assert_eq!(payload.status_code, status);
    assert_eq!(payload.message, "boom");
    assert_eq!(payload.model.as_deref(), Some(SMALL));
}
#[rstest]
#[case::missing_key(OperationError::MissingApiKey, "APIConnectionError", Some(500_u16))]
#[case::timeout(OperationError::Timeout, "Timeout", Some(408_u16))]
#[case::no_adapter(OperationError::UnavailableAdapter { provider: String::from("openai") }, "InternalServerError", Some(500_u16))]
fn error_operation_failures_map_to_litellm_classes(
    #[case] error: OperationError,
    #[case] name: &str,
    #[case] status: Option<u16>,
) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let payload = payload_of(operation_failure(error, &facts(), &redactor));
    assert_eq!(payload.kind, name);
    assert_eq!(payload.status_code, status);
    assert_eq!(payload.model.as_deref(), Some(SMALL));
    assert_eq!(payload.llm_provider.as_deref(), Some("openai"));
}
#[test]
fn error_a_provider_401_is_the_authentication_class() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let response = UpstreamResponse {
        status: 401_u16,
        headers: HeaderMap::new(),
        body: UNAUTHORIZED_BODY.as_bytes().to_vec(),
    };
    let error = OperationError::Provider {
        response: Box::new(response),
    };
    let payload = payload_of(operation_failure(error, &facts(), &redactor));
    assert_eq!(payload.kind, "AuthenticationError");
    assert_eq!(payload.status_code, Some(401_u16));
    assert!(
        payload.message.contains("Invalid API key provided"),
        "{}",
        payload.message
    );
}
#[test]
fn error_an_unknown_model_is_a_bad_request() {
    let wire: EmbeddingCallWire =
        parse(r#"{"arguments": {"model": "no-such-model", "input": "hi"}}"#).unwrap();
    let failure = get_runtime()
        .block_on(call(&session(), wire, Entry::Sync))
        .unwrap_err();
    let payload = payload_of(failure);
    assert_eq!(payload.kind, "BadRequestError");
    assert_eq!(payload.model.as_deref(), Some("no-such-model"));
}
#[test]
fn error_a_mock_error_text_is_a_plain_exception() {
    let wire: EmbeddingCallWire = parse(
        r#"{"arguments": {"model": "text-embedding-3-small", "input": "hi", "mock_response": "error"}}"#,
    )
    .unwrap();
    let failure = get_runtime()
        .block_on(call(&session(), wire, Entry::Sync))
        .unwrap_err();
    let payload = payload_of(failure);
    assert_eq!(payload.kind, "Exception");
    assert_eq!(payload.message, "Mock error");
}
#[test]
fn error_an_awaited_mock_error_is_the_connection_class() {
    let wire: EmbeddingCallWire = parse(
        r#"{"arguments": {"model": "text-embedding-3-small", "input": "hi", "mock_response": "error"}}"#,
    )
    .unwrap();
    let failure = get_runtime()
        .block_on(call(&session(), wire, Entry::Async))
        .unwrap_err();
    let payload = payload_of(failure);
    assert_eq!(payload.kind, "APIConnectionError");
    assert_eq!(payload.status_code, Some(500_u16));
    assert_eq!(
        payload.message,
        "APIConnectionError: OpenAIException - Mock error"
    );
    assert_eq!(payload.model.as_deref(), Some(SMALL));
    assert_eq!(payload.llm_provider.as_deref(), Some("openai"));
}
#[test]
fn request_a_mock_answer_skips_the_provider() {
    let answer = mock_call(Entry::Sync);
    assert_eq!(answer.object, "list");
    assert_eq!(answer.model, SMALL);
    assert_eq!(
        answer.data.iter().map(text_of).collect::<Vec<String>>(),
        [r#"{"object":"embedding","index":0,"embedding":[0.5]}"#]
    );
    assert_eq!(
        text_of(&answer.usage),
        r#"{"prompt_tokens":10,"completion_tokens":0,"total_tokens":0}"#
    );
    assert!(answer.response_headers.is_empty());
    assert!(!answer.hidden_params.contains_key("custom_llm_provider"));
    assert!(
        !answer
            .hidden_params
            .contains_key("litellm_overhead_time_ms")
    );
    assert_eq!(
        answer.hidden_params.get("api_base"),
        Some(&Json::Text(String::from("https://api.openai.com")))
    );
    assert_ne!(answer.hidden_params.get("response_cost"), Some(&Json::Null));
}
#[test]
fn request_an_awaited_mock_answer_names_the_provider() {
    let answer = mock_call(Entry::Async);
    assert_eq!(answer.hidden_params.len(), 1_usize);
    assert_eq!(
        answer.hidden_params.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
}
#[test]
fn request_the_caller_model_and_model_info_reach_a_mock_answer() {
    let answer = mock_answer_of(
        r#"{"model": "openai/text-embedding-3-small", "input": "hi", "mock_response": [0.5],
            "model_info": {"id": "deployment-1"}}"#,
    );
    assert_eq!(answer.model, SMALL);
    assert_eq!(
        answer.hidden_params.get("litellm_model_name"),
        Some(&Json::Text(String::from("openai/text-embedding-3-small")))
    );
    assert_eq!(
        answer.hidden_params.get("model_id"),
        Some(&Json::Text(String::from("deployment-1")))
    );
}
#[test]
fn request_a_provider_answer_sends_the_call_and_keeps_the_headers() {
    let (sent, outcome) = served_call("200 OK", EMBEDDING_BODY, "sk-test");
    let answer = outcome.unwrap();
    assert!(sent.starts_with("POST /v1/embeddings "), "{sent}");
    assert!(sent.contains("x-custom: one"), "{sent}");
    assert!(
        sent.to_lowercase()
            .contains("authorization: bearer sk-test"),
        "{sent}"
    );
    assert!(sent.contains(r#""input":"hi""#), "{sent}");
    assert_eq!(answer.model, SMALL);
    assert_eq!(
        answer.data.iter().map(text_of).collect::<Vec<String>>(),
        [r#"{"object":"embedding","index":0,"embedding":[0.1,0.2]}"#]
    );
    assert_eq!(
        answer
            .response_headers
            .get("x-ratelimit-remaining-requests")
            .map(String::as_str),
        Some("9")
    );
}
#[test]
fn request_a_provider_answer_carries_the_hidden_params() {
    let (_sent, outcome) = served_call("200 OK", EMBEDDING_BODY, "sk-test");
    let answer = outcome.unwrap();
    let hidden = &answer.hidden_params;
    assert_eq!(
        hidden.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
    assert_eq!(
        hidden.get("additional_headers").map(text_of).as_deref(),
        Some("{}")
    );
    let Some(Json::Text(api_base)) = hidden.get("api_base") else {
        panic!("the hidden params carry no api base");
    };
    assert!(api_base.starts_with("http://127.0.0.1:"), "{api_base}");
    assert!(api_base.ends_with("/v1"), "{api_base}");
    let Some(Json::Number(overhead)) = hidden.get("litellm_overhead_time_ms") else {
        panic!("a provider answer carries its overhead");
    };
    let Some(Json::Number(total)) = hidden.get("_response_ms") else {
        panic!("a provider answer carries its response time");
    };
    assert!(overhead.as_f64() <= total.as_f64());
    assert_ne!(hidden.get("response_cost"), Some(&Json::Null));
}
#[test]
fn request_the_environment_authorizes_and_locates_the_call() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    let keyed = session_from(&format!(
        r#"{{"environment": {{"OPENAI_BASE_URL": "http://{address}/v1", "OPENAI_API_KEY": "sk-env"}}}}"#
    ));
    let (sent, outcome) = served_in(
        &listener,
        &keyed,
        "200 OK",
        EMBEDDING_BODY,
        r#"{"model": "text-embedding-3-small", "input": "hi", "timeout": 5}"#,
    );
    outcome.unwrap();
    assert!(sent.starts_with("POST /v1/embeddings "), "{sent}");
    assert!(
        sent.to_lowercase().contains("authorization: bearer sk-env"),
        "{sent}"
    );
}
#[test]
fn error_a_served_401_raises_the_authentication_class() {
    let (_sent, outcome) = served_call("401 Unauthorized", UNAUTHORIZED_BODY, "bad-key");
    let payload = payload_of(outcome.unwrap_err());
    assert_eq!(payload.kind, "AuthenticationError");
    assert_eq!(payload.status_code, Some(401_u16));
    assert_eq!(payload.model.as_deref(), Some(SMALL));
    assert_eq!(payload.llm_provider.as_deref(), Some("openai"));
}
#[rstest]
#[case::azure_flag(r#"{"model": "text-embedding-3-small", "azure": true}"#, "azure")]
#[case::azure_flag_beats_the_provider(
    r#"{"model": "text-embedding-3-small", "azure": true, "custom_llm_provider": "openai"}"#,
    "azure"
)]
#[case::azure_false_keeps_the_provider(
    r#"{"model": "text-embedding-3-small", "azure": false, "custom_llm_provider": "azure"}"#,
    "azure"
)]
#[case::no_flag_infers_openai(r#"{"model": "text-embedding-3-small"}"#, "openai")]
#[case::azure_false_infers_openai(
    r#"{"model": "text-embedding-3-small", "azure": false}"#,
    "openai"
)]
fn request_the_azure_flag_routes_resolution(#[case] text: &str, #[case] provider: &str) {
    let resolved = session()
        .resolve(&arguments(text), None, &GlobalCredentials::default())
        .unwrap();
    assert_eq!(resolved.provider.slug(), provider);
}
#[test]
fn request_global_credentials_skip_empty_settings() {
    let empty: EmbeddingSettingsWire = parse(r#"{"api_base": "", "api_key": ""}"#).unwrap();
    let unset = global_credentials(&empty);
    assert_eq!(unset.api_base, None);
    assert!(unset.api_key.is_none());
    let given: EmbeddingSettingsWire =
        parse(r#"{"api_base": "http://proxy.test", "api_key": "sk-module"}"#).unwrap();
    let globals = global_credentials(&given);
    assert_eq!(globals.api_base.as_deref(), Some("http://proxy.test"));
    assert_eq!(
        globals.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-module")
    );
}
#[test]
fn request_an_empty_api_base_counts_as_unset() {
    let resolved = session()
        .resolve(
            &arguments(r#"{"model": "text-embedding-3-small", "api_base": ""}"#),
            None,
            &GlobalCredentials::default(),
        )
        .unwrap();
    assert_eq!(resolved.api_base, None);
}
fn mock_call(entry: Entry) -> EmbeddingAnswer {
    let wire: EmbeddingCallWire = parse(
        r#"{"arguments": {"model": "text-embedding-3-small", "input": "hi", "mock_response": [0.5], "litellm_call_id": "call-1"}}"#,
    )
    .unwrap();
    get_runtime()
        .block_on(call(&session(), wire, entry))
        .unwrap()
}
// a request is complete once its headers ended and the content-length bytes after them arrived
fn complete(received: &[u8]) -> bool {
    let text = str::from_utf8(received).unwrap();
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0_usize);
    body.len() >= length
}
// one HTTP exchange on the local port: the request text it read, after writing the given answer
fn answer_once(listener: &TcpListener, status: &str, body: &str) -> String {
    let (mut stream, _peer) = listener.accept().unwrap();
    let mut received: Vec<u8> = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !complete(&received) {
        let read = stream.read(&mut chunk).unwrap();
        assert_ne!(read, 0_usize, "the client closed before the request ended");
        received.extend_from_slice(&chunk[..read]);
    }
    let reply = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\nx-ratelimit-remaining-requests: 9\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(reply.as_bytes()).unwrap();
    String::from_utf8(received).unwrap()
}
fn served_in(
    listener: &TcpListener,
    embedding_session: &EmbeddingSession,
    status: &str,
    body: &str,
    arguments_json: &str,
) -> (String, Result<EmbeddingAnswer, CallFailure>) {
    let address: SocketAddr = listener.local_addr().unwrap();
    let wire: EmbeddingCallWire = parse(&format!(r#"{{"arguments": {arguments_json}}}"#)).unwrap();
    thread::scope(|scope| {
        let server = scope.spawn(|| answer_once(listener, status, body));
        let outcome = get_runtime().block_on(call(embedding_session, wire, Entry::Sync));
        // a call that failed before it connected would leave the server waiting in accept
        let _unblock = TcpStream::connect(address);
        (server.join().unwrap(), outcome)
    })
}
fn served_call(
    status: &str,
    body: &str,
    api_key: &str,
) -> (String, Result<EmbeddingAnswer, CallFailure>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    let arguments_json = format!(
        r#"{{"model": "text-embedding-3-small", "input": "hi", "api_key": "{api_key}", "api_base": "http://{address}/v1", "timeout": 5, "headers": {{"x-custom": "one"}}}}"#
    );
    served_in(&listener, &session(), status, body, &arguments_json)
}
