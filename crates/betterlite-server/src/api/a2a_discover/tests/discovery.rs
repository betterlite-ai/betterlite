use std::collections::BTreeMap;
use std::fs::read_to_string;
use std::sync::Arc;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;
use tokio::runtime::Builder;
use tracing::subscriber::with_default;

use crate::api::a2a_discover::discovery::{
    AgentCardDiscoveryError, DEFAULT_DISCOVERY_TIMEOUT, DiscoveryMode, fetch_well_known_card,
};
use crate::api::a2a_discover::source::{LiveDiscovery, SafeCardSource};
use crate::api::a2a_discover::{DiscoverAgentCard as _, DiscoverAgentRequest};

use super::support::{Reply, ScriptedReply, ScriptedSource, Server, proxied_client, safe_fetcher};

const CARD: &str = r#"{"name":"agent","skills":[{"id":"echo"}]}"#;

fn card() -> AgentJson {
    serde_json::from_str(CARD).unwrap()
}

fn assistant_params(id: &str) -> BTreeMap<String, AgentJson> {
    BTreeMap::from([("assistant_id".to_owned(), AgentJson::String(id.to_owned()))])
}

fn responses(bodies: &[(u16, &str)]) -> ScriptedSource {
    ScriptedSource::new(
        bodies
            .iter()
            .map(|(status, body)| ScriptedReply::Response {
                status: *status,
                body: (*body).to_owned(),
            })
            .collect(),
    )
}

#[tokio::test]
async fn well_known_fallback_tries_the_three_paths_in_order() {
    let source = ScriptedSource::failing(3);
    let error = fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        source.urls(),
        [
            "https://agent.test/.well-known/agent-card.json",
            "https://agent.test/.well-known/agent.json",
            "https://agent.test/agent.json",
        ]
    );
    assert!(error.to_string().contains(
        "Last error: https://agent.test/agent.json: the request did not finish within 10s"
    ));
}

#[tokio::test]
async fn langgraph_platform_appends_the_assistant_id_to_every_path() {
    let source = ScriptedSource::failing(3);
    let params = assistant_params("asst 1");
    fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::LanggraphPlatform,
        &params,
    )
    .await
    .unwrap_err();
    assert_eq!(
        source.urls(),
        [
            "https://agent.test/.well-known/agent-card.json?assistant_id=asst+1",
            "https://agent.test/.well-known/agent.json?assistant_id=asst+1",
            "https://agent.test/agent.json?assistant_id=asst+1",
        ]
    );
}

#[rstest]
#[case::empty(BTreeMap::new())]
#[case::without_the_key(BTreeMap::from([(
    "other".to_owned(),
    AgentJson::Bool(true),
)]))]
#[case::null(BTreeMap::from([("assistant_id".to_owned(), AgentJson::Null)]))]
#[tokio::test]
async fn langgraph_platform_without_an_assistant_id_fails_at_once(
    #[case] params: BTreeMap<String, AgentJson>,
) {
    let source = ScriptedSource::failing(3);
    let error = fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::LanggraphPlatform,
        &params,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "langgraph_platform discovery requires params.assistant_id"
    );
    assert!(source.urls().is_empty());
}

#[tokio::test]
async fn an_empty_base_url_fails_at_once() {
    let source = ScriptedSource::failing(3);
    let error = fetch_well_known_card(
        &source,
        "",
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "base_url is required");
    assert!(source.urls().is_empty());
}

#[tokio::test]
async fn a_trailing_slash_on_the_base_url_is_dropped() {
    let source = ScriptedSource::failing(3);
    fetch_well_known_card(
        &source,
        "https://agent.test///",
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        source.urls(),
        [
            "https://agent.test/.well-known/agent-card.json",
            "https://agent.test/.well-known/agent.json",
            "https://agent.test/agent.json",
        ]
    );
}

#[tokio::test]
async fn the_first_card_below_four_hundred_wins() {
    let source = responses(&[(404, "not found"), (200, CARD)]);
    let fetched = fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(fetched, card());
    assert_eq!(
        source.urls(),
        [
            "https://agent.test/.well-known/agent-card.json",
            "https://agent.test/.well-known/agent.json",
        ]
    );
}

#[tokio::test]
async fn a_non_json_body_is_skipped() {
    let source = responses(&[(200, "not json"), (200, CARD)]);
    let fetched = fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(fetched, card());
    assert_eq!(source.urls().len(), 2);
}

#[rstest]
#[case::array("[1,2]")]
#[case::string("\"text\"")]
#[case::number("1")]
#[case::boolean("true")]
#[case::null("null")]
#[tokio::test]
async fn a_json_value_that_is_not_an_object_is_skipped(#[case] body: &str) {
    let source = responses(&[(200, body), (200, CARD)]);
    let fetched = fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(fetched, card());
    assert_eq!(source.urls().len(), 2);
}

#[tokio::test]
async fn the_last_error_names_the_json_error() {
    let source = responses(&[(200, "not json"), (200, "not json"), (200, "not json")]);
    let error = fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("https://agent.test/agent.json: invalid JSON ("),
        "{error}"
    );
}

#[rstest]
#[case::array("[1,2]", "an array")]
#[case::string("\"text\"", "a string")]
#[case::number("1", "a number")]
#[case::boolean("true", "a boolean")]
#[case::null("null", "null")]
#[tokio::test]
async fn the_last_error_names_the_json_kind_it_rejected(#[case] body: &str, #[case] kind: &str) {
    let source = responses(&[(200, body), (200, body), (200, body)]);
    let error = fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "Could not fetch agent card from https://agent.test (mode=well_known_fallback). Last error: https://agent.test/agent.json: expected a JSON object, got {kind}"
        )
    );
}

#[test]
fn the_default_discovery_timeout_is_ten_seconds() {
    assert_eq!(DEFAULT_DISCOVERY_TIMEOUT, Duration::from_secs(10));
}

#[test]
fn every_attempt_is_logged_at_debug() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let source = responses(&[(404, "not found"), (200, CARD)]);
    with_default(subscriber, || {
        let fetched = runtime
            .block_on(fetch_well_known_card(
                &source,
                "https://agent.test",
                DiscoveryMode::WellKnownFallback,
                &BTreeMap::new(),
            ))
            .unwrap();
        assert_eq!(fetched, card());
    });
    let output = read_to_string(log.path()).unwrap();
    assert!(output.contains("A2A discovery path failed"));
    assert!(output.contains("https://agent.test/.well-known/agent-card.json"));
    assert!(output.contains("A2A discovery succeeded"));
    assert!(output.contains("https://agent.test/.well-known/agent.json"));
}

#[tokio::test]
async fn a_status_of_three_hundred_ninety_nine_is_still_read() {
    let source = responses(&[(399, CARD)]);
    let fetched = fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(fetched, card());
    assert_eq!(source.urls().len(), 1);
}

#[tokio::test]
async fn a_status_of_four_hundred_is_not_read() {
    let source = responses(&[(400, CARD)]);
    fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(source.urls().len(), 3);
}

#[tokio::test]
async fn a_blocked_path_moves_to_the_next_one() {
    let source = ScriptedSource::new(vec![
        ScriptedReply::Blocked,
        ScriptedReply::Response {
            status: 200,
            body: CARD.to_owned(),
        },
    ]);
    let fetched = fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(fetched, card());
    assert_eq!(source.urls().len(), 2);
}

#[tokio::test]
async fn a_failed_request_moves_to_the_next_one() {
    let source = ScriptedSource::new(vec![
        ScriptedReply::Failure,
        ScriptedReply::Response {
            status: 200,
            body: CARD.to_owned(),
        },
    ]);
    let fetched = fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(fetched, card());
    assert_eq!(source.urls().len(), 2);
}

#[tokio::test]
async fn the_error_names_the_base_url_the_mode_and_the_last_failure() {
    let source = responses(&[(500, ""), (500, ""), (500, "")]);
    let params = assistant_params("a1");
    let error = fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::LanggraphPlatform,
        &params,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error,
        AgentCardDiscoveryError::AllPathsFailed {
            base_url: "https://agent.test".to_owned(),
            mode: DiscoveryMode::LanggraphPlatform,
            last_error: "https://agent.test/agent.json?assistant_id=a1: HTTP 500".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "Could not fetch agent card from https://agent.test (mode=langgraph_platform). Last error: https://agent.test/agent.json?assistant_id=a1: HTTP 500"
    );
}

#[tokio::test]
async fn a_loopback_url_is_blocked_before_any_request() {
    let client = proxied_client(Duration::from_secs(5));
    let urls = safe_fetcher(&[]);
    let source = SafeCardSource {
        client: &client,
        urls: &urls,
        timeout: Duration::from_secs(5),
    };
    let error = fetch_well_known_card(
        &source,
        "http://127.0.0.1:9",
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("http://127.0.0.1:9/agent.json: URL targets a blocked address (127.0.0.1)")
    );
}

#[tokio::test]
async fn a_redirect_to_a_blocked_host_is_not_followed() {
    let server = Server::start(vec![
        (
            "/.well-known/agent-card.json",
            Reply::redirect("http://10.1.2.3/.well-known/agent-card.json"),
        ),
        ("/.well-known/agent.json", Reply::json(CARD)),
    ])
    .await;
    let client = proxied_client(Duration::from_secs(5));
    let urls = safe_fetcher(&["127.0.0.1"]);
    let source = SafeCardSource {
        client: &client,
        urls: &urls,
        timeout: Duration::from_secs(5),
    };
    let fetched = fetch_well_known_card(
        &source,
        &server.url(""),
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(fetched, card());
    assert_eq!(
        server.requests(),
        ["/.well-known/agent-card.json", "/.well-known/agent.json"]
    );
}

#[tokio::test]
async fn a_slow_answer_is_cut_at_the_deadline() {
    let server = Server::start(vec![
        (
            "/.well-known/agent-card.json",
            Reply::delayed(Duration::from_secs(3), CARD),
        ),
        ("/.well-known/agent.json", Reply::json(CARD)),
    ])
    .await;
    let client = proxied_client(Duration::from_secs(5));
    let urls = safe_fetcher(&["127.0.0.1"]);
    let source = SafeCardSource {
        client: &client,
        urls: &urls,
        timeout: Duration::from_millis(300),
    };
    let fetched = fetch_well_known_card(
        &source,
        &server.url(""),
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(fetched, card());
    assert_eq!(
        server.requests(),
        ["/.well-known/agent-card.json", "/.well-known/agent.json"]
    );
}

#[tokio::test]
async fn a_stalled_body_is_cut_at_the_deadline() {
    let server = Server::start(vec![
        (
            "/.well-known/agent-card.json",
            Reply::stalled(Duration::from_secs(3), CARD),
        ),
        ("/.well-known/agent.json", Reply::json(CARD)),
    ])
    .await;
    let client = proxied_client(Duration::from_secs(5));
    let urls = safe_fetcher(&["127.0.0.1"]);
    let source = SafeCardSource {
        client: &client,
        urls: &urls,
        timeout: Duration::from_millis(300),
    };
    let fetched = fetch_well_known_card(
        &source,
        &server.url(""),
        DiscoveryMode::WellKnownFallback,
        &BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(fetched, card());
    assert_eq!(
        server.requests(),
        ["/.well-known/agent-card.json", "/.well-known/agent.json"]
    );
}

#[tokio::test]
async fn live_discovery_returns_the_card_for_a_langgraph_request() {
    let server = Server::start(vec![("/.well-known/agent.json", Reply::json(CARD))]).await;
    let discovery = LiveDiscovery {
        client: proxied_client(Duration::from_secs(5)),
        urls: safe_fetcher(&["127.0.0.1"]),
        timeout: Duration::from_secs(5),
    };
    let request = DiscoverAgentRequest {
        url: server.url(""),
        discovery_mode: DiscoveryMode::LanggraphPlatform,
        params: assistant_params("a1"),
    };
    let fetched = discovery.discover(&request).await.unwrap();
    assert_eq!(fetched, card());
    assert_eq!(
        server.requests(),
        [
            "/.well-known/agent-card.json?assistant_id=a1",
            "/.well-known/agent.json?assistant_id=a1",
        ]
    );
}

#[tokio::test]
async fn live_discovery_reports_the_discovery_error_when_every_path_fails() {
    let server = Server::start(Vec::new()).await;
    let discovery = LiveDiscovery {
        client: proxied_client(Duration::from_secs(5)),
        urls: safe_fetcher(&["127.0.0.1"]),
        timeout: Duration::from_secs(5),
    };
    let request = DiscoverAgentRequest {
        url: server.url(""),
        discovery_mode: DiscoveryMode::WellKnownFallback,
        params: BTreeMap::new(),
    };
    let failure = discovery.discover(&request).await.unwrap_err();
    let message = failure.to_string();
    assert!(
        message.contains("Could not fetch agent card from"),
        "{message}"
    );
    assert!(message.contains("/agent.json: HTTP 404"), "{message}");
}
