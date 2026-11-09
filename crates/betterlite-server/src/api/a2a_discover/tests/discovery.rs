use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use crate::api::a2a_discover::discovery::{
    AgentCardDiscoveryError, DEFAULT_DISCOVERY_TIMEOUT, DiscoveryMode, fetch_well_known_card,
};
use crate::api::a2a_discover::source::SafeCardSource;

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
        None,
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
        Some(&params),
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
#[case::absent(None)]
#[case::empty(Some(BTreeMap::new()))]
#[case::null(Some(BTreeMap::from([(
    "assistant_id".to_owned(),
    AgentJson::Null,
)])))]
#[tokio::test]
async fn langgraph_platform_without_an_assistant_id_fails_at_once(
    #[case] params: Option<BTreeMap<String, AgentJson>>,
) {
    let source = ScriptedSource::failing(3);
    let error = fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::LanggraphPlatform,
        params.as_ref(),
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
    let error = fetch_well_known_card(&source, "", DiscoveryMode::WellKnownFallback, None)
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
        None,
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
        None,
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
        None,
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
        None,
    )
    .await
    .unwrap();
    assert_eq!(fetched, card());
    assert_eq!(source.urls().len(), 2);
}

#[tokio::test]
async fn the_last_error_names_the_json_kind_it_rejected() {
    let source = responses(&[(200, "[1]"), (200, "\"text\""), (200, "null")]);
    let error = fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::WellKnownFallback,
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Could not fetch agent card from https://agent.test (mode=well_known_fallback). Last error: https://agent.test/agent.json: expected a JSON object, got null"
    );
}

#[test]
fn the_default_discovery_timeout_is_ten_seconds() {
    assert_eq!(DEFAULT_DISCOVERY_TIMEOUT, Duration::from_secs(10));
}

#[tokio::test]
async fn a_status_of_three_hundred_ninety_nine_is_still_read() {
    let source = responses(&[(399, CARD)]);
    let fetched = fetch_well_known_card(
        &source,
        "https://agent.test",
        DiscoveryMode::WellKnownFallback,
        None,
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
        None,
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
        None,
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
        None,
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
        Some(&params),
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
        None,
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
        None,
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
            Reply::delayed(Duration::from_millis(300), CARD),
        ),
        ("/.well-known/agent.json", Reply::json(CARD)),
    ])
    .await;
    let client = proxied_client(Duration::from_secs(5));
    let urls = safe_fetcher(&["127.0.0.1"]);
    let source = SafeCardSource {
        client: &client,
        urls: &urls,
        timeout: Duration::from_millis(50),
    };
    let fetched = fetch_well_known_card(
        &source,
        &server.url(""),
        DiscoveryMode::WellKnownFallback,
        None,
    )
    .await
    .unwrap();
    assert_eq!(fetched, card());
    assert_eq!(
        server.requests(),
        ["/.well-known/agent-card.json", "/.well-known/agent.json"]
    );
}
