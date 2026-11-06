#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use thiserror::Error;
use tracing::subscriber::{DefaultGuard, set_default};

use super::{
    A2AException, A2AExceptionCheckers, A2aSdk, CardUrls, ClientConfig, ClientStash,
    LocalhostUrlError, RecoveryError, handle_a2a_localhost_retry, map_a2a_exception,
};

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_warnings() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FakeCard {
    url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FakeCallContext(&'static str);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct FakeClient {
    httpx_client: Option<&'static str>,
    call_context: Option<FakeCallContext>,
    config: Option<(&'static str, bool, String)>,
    stashed_card: Option<FakeCard>,
}

#[derive(Debug, Error)]
#[error("sdk refused the card")]
struct FakeSdkError;

#[derive(Default)]
struct FakeSdk<const AVAILABLE: bool> {
    events: Mutex<Vec<&'static str>>,
    fail_create: bool,
}

impl<const AVAILABLE: bool> FakeSdk<AVAILABLE> {
    fn record(&self, event: &'static str) {
        self.events.lock().unwrap().push(event);
    }

    fn events(&self) -> Vec<&'static str> {
        self.events.lock().unwrap().clone()
    }
}

impl<const AVAILABLE: bool> CardUrls for FakeSdk<AVAILABLE> {
    type Card = FakeCard;

    fn is_localhost_or_internal_url(&self, url: Option<&str>) -> bool {
        url.is_some_and(|url| {
            ["localhost", "0.0.0.0", "internal-marker"]
                .iter()
                .any(|marker| url.contains(marker))
        })
    }

    fn set_agent_card_url(&self, card: &mut FakeCard, url: &str) {
        self.record("set_agent_card_url");
        card.url = format!("{}/", url.trim_end_matches('/'));
    }
}

impl<const AVAILABLE: bool> A2aSdk for FakeSdk<AVAILABLE> {
    const A2A_SDK_AVAILABLE: bool = AVAILABLE;

    type Client = FakeClient;
    type HttpClient = &'static str;
    type CallContext = FakeCallContext;
    type Error = FakeSdkError;

    fn httpx_client(&self, client: &FakeClient) -> Option<&'static str> {
        self.record("httpx_client");
        client.httpx_client
    }

    fn call_context(&self, client: &FakeClient) -> Option<FakeCallContext> {
        client.call_context.clone()
    }

    async fn create_client(
        &self,
        card: &FakeCard,
        config: ClientConfig<&'static str>,
    ) -> Result<FakeClient, FakeSdkError> {
        self.record("create_client");
        if self.fail_create {
            return Err(FakeSdkError);
        }
        Ok(FakeClient {
            httpx_client: None,
            call_context: None,
            config: Some((config.httpx_client, config.streaming, card.url.clone())),
            stashed_card: None,
        })
    }

    fn attach_stash(
        &self,
        client: &mut FakeClient,
        stash: ClientStash<&'static str, FakeCallContext, FakeCard>,
    ) {
        client.httpx_client = Some(stash.httpx_client);
        client.call_context = stash.call_context;
        client.stashed_card = Some(stash.agent_card);
    }
}

fn localhost_error() -> LocalhostUrlError {
    LocalhostUrlError {
        localhost_url: "http://localhost:8001/".to_owned(),
        base_url: "https://agents.example.com".to_owned(),
        original_error: "connection refused".to_owned(),
        model: Some("agent-1".to_owned()),
    }
}

fn client_with_stash() -> FakeClient {
    FakeClient {
        httpx_client: Some("pooled"),
        call_context: Some(FakeCallContext("trace-1")),
        config: None,
        stashed_card: None,
    }
}

fn card() -> FakeCard {
    FakeCard {
        url: "http://localhost:8001/".to_owned(),
    }
}

#[rstest]
#[case("Failed to CONNECT to host", true)]
#[case("Connection reset by peer", true)]
#[case("Network is unreachable", true)]
#[case("connect: Refused", true)]
#[case("401 unauthorized", false)]
#[case("", false)]
fn exception_checkers_connection(#[case] error: &str, #[case] expected: bool) {
    assert_eq!(A2AExceptionCheckers::is_connection_error(error), expected);
}

#[rstest]
#[case("Failed to fetch Agent Card", true)]
#[case("bad agent-card body", true)]
#[case("GET /.WELL-KNOWN/agent.json failed", true)]
#[case("Card Not Found", true)]
#[case("Invalid Agent definition", true)]
#[case("agent timed out", false)]
#[case("", false)]
fn exception_checkers_card(#[case] error: &str, #[case] expected: bool) {
    assert_eq!(A2AExceptionCheckers::is_agent_card_error(error), expected);
}

#[rstest]
#[case::sdk_marks_url_internal("https://internal-marker.example.com/", true)]
#[case::sdk_marks_url_public("https://public.example.com/", false)]
fn exception_checkers_localhost(#[case] card_url: &str, #[case] expected: bool) {
    let sdk = FakeSdk::<true>::default();
    let mapped = map_a2a_exception(
        &sdk,
        &"connection refused",
        Some(card_url),
        Some("https://agents.example.com"),
        None,
    );
    assert_eq!(matches!(mapped, A2AException::LocalhostUrl(_)), expected);
}

#[rstest]
#[case::localhost_connection(
    "connection refused",
    Some("http://localhost:8001/"),
    Some("https://agents.example.com"),
    A2AException::LocalhostUrl(LocalhostUrlError {
        localhost_url: "http://localhost:8001/".to_owned(),
        base_url: "https://agents.example.com".to_owned(),
        original_error: "connection refused".to_owned(),
        model: Some("agent-1".to_owned()),
    }),
)]
#[case::localhost_beats_card_error(
    "Agent Card connection refused",
    Some("http://0.0.0.0:8001/"),
    Some("https://agents.example.com"),
    A2AException::LocalhostUrl(LocalhostUrlError {
        localhost_url: "http://0.0.0.0:8001/".to_owned(),
        base_url: "https://agents.example.com".to_owned(),
        original_error: "Agent Card connection refused".to_owned(),
        model: Some("agent-1".to_owned()),
    }),
)]
#[case::card_beats_connection(
    "Agent Card connection refused",
    Some("https://public.example.com/"),
    Some("https://agents.example.com"),
    A2AException::AgentCard {
        message: "Agent Card connection refused".to_owned(),
        url: Some("https://agents.example.com".to_owned()),
        model: Some("agent-1".to_owned()),
    },
)]
#[case::localhost_without_api_base_is_connection(
    "connection refused",
    Some("http://localhost:8001/"),
    None,
    A2AException::Connection {
        message: "connection refused".to_owned(),
        url: Some("http://localhost:8001/".to_owned()),
        model: Some("agent-1".to_owned()),
    },
)]
#[case::localhost_with_other_error_is_other(
    "boom",
    Some("http://localhost:8001/"),
    Some("https://agents.example.com"),
    A2AException::Other {
        message: "boom".to_owned(),
        model: Some("agent-1".to_owned()),
    },
)]
#[case::connection_falls_back_to_api_base(
    "network down",
    None,
    Some("https://agents.example.com"),
    A2AException::Connection {
        message: "network down".to_owned(),
        url: Some("https://agents.example.com".to_owned()),
        model: Some("agent-1".to_owned()),
    },
)]
fn exception_mapping_order(
    #[case] error: &str,
    #[case] card_url: Option<&str>,
    #[case] api_base: Option<&str>,
    #[case] expected: A2AException,
) {
    let sdk = FakeSdk::<true>::default();
    let mapped = map_a2a_exception(&sdk, &error, card_url, api_base, Some("agent-1"));
    assert_eq!(mapped, expected);
}

#[test]
fn exception_mapping_accepts_any_display_error() {
    let sdk = FakeSdk::<true>::default();
    let mapped = map_a2a_exception(&sdk, &io::Error::other("boom"), None, None, None);
    assert_eq!(
        mapped,
        A2AException::Other {
            message: "boom".to_owned(),
            model: None
        }
    );
}

#[tokio::test]
async fn exception_recovery_missing_card() {
    let sdk = FakeSdk::<true>::default();
    let result =
        handle_a2a_localhost_retry(&sdk, &localhost_error(), None, &client_with_stash(), false)
            .await;
    let error = result.unwrap_err();
    assert!(matches!(error, RecoveryError::MissingAgentCard));
    assert!(
        error
            .to_string()
            .contains("no agent card is available to rewrite")
    );
    assert!(sdk.events().is_empty());
}

#[tokio::test]
async fn exception_recovery_missing_card_logs_nothing() {
    let (captured, _guard) = capture_warnings();
    let sdk = FakeSdk::<true>::default();
    handle_a2a_localhost_retry(&sdk, &localhost_error(), None, &client_with_stash(), true)
        .await
        .unwrap_err();
    assert_eq!(captured.text(), "");
}

#[rstest]
#[case::no_stash(FakeClient::default())]
#[case::only_context(FakeClient { httpx_client: None, call_context: Some(FakeCallContext("trace-1")), config: None, stashed_card: None })]
#[tokio::test]
async fn exception_recovery_external_client(#[case] client: FakeClient) {
    let sdk = FakeSdk::<true>::default();
    let mut card = card();
    let error =
        handle_a2a_localhost_retry(&sdk, &localhost_error(), Some(&mut card), &client, false)
            .await
            .unwrap_err();
    assert!(matches!(error, RecoveryError::ExternalClient));
    assert!(
        error
            .to_string()
            .contains("was not created by create_a2a_client")
    );
    assert_eq!(sdk.events(), ["set_agent_card_url", "httpx_client"]);
}

#[rstest]
#[case(
    false,
    "A2A request to 'http://localhost:8001/' failed: connection refused."
)]
#[case(
    true,
    "A2A streaming request to 'http://localhost:8001/' failed: connection refused."
)]
#[tokio::test]
async fn exception_recovery_warning(#[case] is_streaming: bool, #[case] expected: &str) {
    let (captured, _guard) = capture_warnings();
    let sdk = FakeSdk::<true>::default();
    let mut card = card();
    handle_a2a_localhost_retry(
        &sdk,
        &localhost_error(),
        Some(&mut card),
        &client_with_stash(),
        is_streaming,
    )
    .await
    .unwrap();
    let text = captured.text();
    assert!(text.contains("WARN"), "{text}");
    assert!(text.contains(expected), "{text}");
    assert!(
        text.contains("Retrying with base_url 'https://agents.example.com'."),
        "{text}"
    );
}

#[tokio::test]
async fn exception_recovery_card_mutation() {
    let sdk = FakeSdk::<true>::default();
    let mut card = card();
    handle_a2a_localhost_retry(
        &sdk,
        &localhost_error(),
        Some(&mut card),
        &client_with_stash(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(card.url, "https://agents.example.com/");
    assert_eq!(
        sdk.events(),
        ["set_agent_card_url", "httpx_client", "create_client"]
    );
}

#[rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn exception_recovery_client_reconstruction(#[case] is_streaming: bool) {
    let sdk = FakeSdk::<true>::default();
    let mut card = card();
    let rebuilt = handle_a2a_localhost_retry(
        &sdk,
        &localhost_error(),
        Some(&mut card),
        &client_with_stash(),
        is_streaming,
    )
    .await
    .unwrap();
    assert_eq!(
        rebuilt.config,
        Some((
            "pooled",
            is_streaming,
            "https://agents.example.com/".to_owned()
        ))
    );
    assert_eq!(rebuilt.httpx_client, Some("pooled"));
    assert_eq!(rebuilt.call_context, Some(FakeCallContext("trace-1")));
    assert_eq!(rebuilt.stashed_card, Some(card));
}

#[tokio::test]
async fn exception_recovery_keeps_missing_call_context() {
    let sdk = FakeSdk::<true>::default();
    let mut card = card();
    let client = FakeClient {
        httpx_client: Some("pooled"),
        call_context: None,
        config: None,
        stashed_card: None,
    };
    let rebuilt =
        handle_a2a_localhost_retry(&sdk, &localhost_error(), Some(&mut card), &client, false)
            .await
            .unwrap();
    assert_eq!(rebuilt.call_context, None);
}

#[tokio::test]
async fn exception_recovery_create_failure_keeps_source() {
    let sdk = FakeSdk::<true> {
        events: Mutex::default(),
        fail_create: true,
    };
    let mut card = card();
    let error = handle_a2a_localhost_retry(
        &sdk,
        &localhost_error(),
        Some(&mut card),
        &client_with_stash(),
        false,
    )
    .await
    .unwrap_err();
    let RecoveryError::CreateClient(source) = error else {
        panic!("expected CreateClient");
    };
    assert_eq!(source.to_string(), "sdk refused the card");
}

#[tokio::test]
async fn exception_recovery_sdk_check() {
    let sdk = FakeSdk::<false>::default();
    let original = card();
    let mut card = original.clone();
    let error = handle_a2a_localhost_retry(
        &sdk,
        &localhost_error(),
        Some(&mut card),
        &client_with_stash(),
        false,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, RecoveryError::SdkUnavailable));
    assert_eq!(
        error.to_string(),
        "A2A SDK is required for localhost retry handling"
    );
    assert!(sdk.events().is_empty());
    assert_eq!(card, original);
}

#[tokio::test]
async fn exception_recovery_sdk_check_runs_before_card_check() {
    let sdk = FakeSdk::<false>::default();
    let error =
        handle_a2a_localhost_retry(&sdk, &localhost_error(), None, &client_with_stash(), false)
            .await
            .unwrap_err();
    assert!(matches!(error, RecoveryError::SdkUnavailable));
}

#[test]
fn localhost_error_message_names_both_urls() {
    assert_eq!(
        localhost_error().to_string(),
        "the agent card points at the internal URL 'http://localhost:8001/'; retry with 'https://agents.example.com'"
    );
}
