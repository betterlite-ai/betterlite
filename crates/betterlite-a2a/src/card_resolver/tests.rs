#![cfg(test)]

use std::collections::BTreeMap;
use std::future::IntoFuture as _;
use std::net::TcpListener as StdTcpListener;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::routing::get;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use http::{HeaderMap, HeaderValue, StatusCode, Uri};
use rstest::rstest;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use super::CardResolver;
use super::card_url::{get_agent_card_url, set_agent_card_url};
use super::interfaces::normalize_interfaces;
use super::localhost::{LOCALHOST_URL_PATTERNS, is_localhost_or_internal_url};
use crate::agent_card::{AgentCard, Interface, ProtocolVersion, WireValue};
use crate::errors::A2AError;

const AGENT_CARD: &str = "/.well-known/agent-card.json";
const AGENT_JSON: &str = "/.well-known/agent.json";
const FOUNDRY: &str = "/agentCard/v1.0";
const CUSTOM: &str = "/custom/card.json";
const FIRST_CARD: &str = r#"{"name":"first"}"#;
const SECOND_CARD: &str = r#"{"name":"second"}"#;
const THIRD_CARD: &str = r#"{"name":"third"}"#;
const CUSTOM_CARD: &str = r#"{"name":"custom"}"#;

type Route = (&'static str, StatusCode, &'static str);

struct Canned {
    routes: Vec<Route>,
    hits: Mutex<Vec<(String, bool)>>,
}

struct Probe {
    result: Result<AgentCard, A2AError>,
    hits: Vec<(String, bool)>,
    base: String,
}

async fn answer(
    State(canned): State<Arc<Canned>>,
    headers: HeaderMap,
    uri: Uri,
) -> (StatusCode, &'static str) {
    let team_sent = headers.contains_key("x-team");
    canned
        .hits
        .lock()
        .unwrap()
        .push((uri.path().to_owned(), team_sent));
    canned
        .routes
        .iter()
        .find(|(path, ..)| *path == uri.path())
        .map_or((StatusCode::NOT_FOUND, "not found"), |(_, status, body)| {
            (*status, *body)
        })
}

fn pooled_client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

async fn probe(routes: Vec<Route>, path: Option<&str>, headers: &HeaderMap) -> Probe {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let canned = Arc::new(Canned {
        routes,
        hits: Mutex::new(Vec::new()),
    });
    let app = Router::new()
        .fallback(get(answer))
        .with_state(Arc::clone(&canned));
    let resolver = CardResolver {
        base_url: base.clone(),
        client: pooled_client(),
    };
    let (stop, stopped) = oneshot::channel::<()>();
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(async move { stopped.await.unwrap_or_default() })
        .into_future();
    let fetch = async {
        let found = resolver.get_agent_card(path, headers).await;
        stop.send(()).unwrap_or_default();
        found
    };
    let (result, outcome) = tokio::join!(fetch, server);
    outcome.unwrap();
    let hits = canned.hits.lock().unwrap().clone();
    Probe { result, hits, base }
}

fn paths(hits: &[(String, bool)]) -> Vec<&str> {
    hits.iter().map(|(path, _)| path.as_str()).collect()
}

fn name_of(card: &AgentCard) -> Option<&str> {
    match card.name.as_ref()?.as_ref()? {
        WireValue::Typed(name) => Some(name.as_str()),
        WireValue::Passthrough(_) => None,
    }
}

fn card_with(fields: &str) -> AgentCard {
    serde_json::from_str(fields).unwrap()
}

fn first_interface(card: &AgentCard) -> &Interface {
    match card.supported_interfaces.as_ref().and_then(Option::as_ref) {
        Some(WireValue::Typed(interfaces)) => interfaces.first().unwrap(),
        _ => panic!("the card has no typed interface list"),
    }
}

fn binding_of(interface: &Interface) -> Option<&str> {
    match interface.protocol_binding.as_ref()?.as_ref()? {
        WireValue::Typed(binding) => Some(binding.as_str()),
        WireValue::Passthrough(_) => None,
    }
}

fn version_of(interface: &Interface) -> Option<ProtocolVersion> {
    match interface.protocol_version.as_ref()?.as_ref()? {
        WireValue::Typed(version) => Some(*version),
        WireValue::Passthrough(_) => None,
    }
}

#[tokio::test]
async fn card_resolver_returns_first_probe_that_succeeds() {
    let probed = probe(
        vec![(AGENT_CARD, StatusCode::OK, FIRST_CARD)],
        None,
        &HeaderMap::new(),
    )
    .await;
    let card = probed.result.unwrap();
    assert_eq!(name_of(&card), Some("first"));
    assert_eq!(paths(&probed.hits), vec![AGENT_CARD]);
}

#[rstest]
#[case::agent_json(
    vec![(AGENT_CARD, StatusCode::NOT_FOUND, "missing"), (AGENT_JSON, StatusCode::OK, SECOND_CARD)],
    vec![AGENT_CARD, AGENT_JSON],
    "second"
)]
#[case::foundry(
    vec![
        (AGENT_CARD, StatusCode::NOT_FOUND, "missing"),
        (AGENT_JSON, StatusCode::NOT_FOUND, "missing"),
        (FOUNDRY, StatusCode::OK, THIRD_CARD),
    ],
    vec![AGENT_CARD, AGENT_JSON, FOUNDRY],
    "third"
)]
#[tokio::test]
async fn card_resolver_falls_through_probes_until_one_succeeds(
    #[case] routes: Vec<Route>,
    #[case] expected_hits: Vec<&str>,
    #[case] expected_name: &str,
) {
    let probed = probe(routes, None, &HeaderMap::new()).await;
    let card = probed.result.unwrap();
    assert_eq!(name_of(&card), Some(expected_name));
    assert_eq!(paths(&probed.hits), expected_hits);
}

#[tokio::test]
async fn card_resolver_supplied_path_bypasses_probing() {
    let routes = vec![
        (AGENT_CARD, StatusCode::OK, FIRST_CARD),
        (CUSTOM, StatusCode::OK, CUSTOM_CARD),
    ];
    let probed = probe(routes, Some(CUSTOM), &HeaderMap::new()).await;
    let card = probed.result.unwrap();
    assert_eq!(name_of(&card), Some("custom"));
    assert_eq!(paths(&probed.hits), vec![CUSTOM]);
}

#[tokio::test]
async fn card_resolver_empty_supplied_path_uses_the_default_card_path() {
    let routes = vec![
        (AGENT_CARD, StatusCode::OK, FIRST_CARD),
        (AGENT_JSON, StatusCode::OK, SECOND_CARD),
    ];
    let probed = probe(routes, Some(""), &HeaderMap::new()).await;
    let card = probed.result.unwrap();
    assert_eq!(name_of(&card), Some("first"));
    assert_eq!(paths(&probed.hits), vec![AGENT_CARD]);
}

#[tokio::test]
async fn card_resolver_supplied_path_failure_names_only_that_path() {
    let probed = probe(Vec::new(), Some(CUSTOM), &HeaderMap::new()).await;
    let error = probed.result.unwrap_err();
    assert_eq!(error.status_code(), 404);
    assert_eq!(
        error.to_string(),
        format!(
            "no agent card found at {}. Tried {CUSTOM} (HTTP 404)",
            probed.base
        )
    );
}

#[rstest]
#[case::second_is_server_error([StatusCode::NOT_FOUND, StatusCode::INTERNAL_SERVER_ERROR, StatusCode::NOT_FOUND], 500)]
#[case::first_non_404_wins([StatusCode::BAD_GATEWAY, StatusCode::SERVICE_UNAVAILABLE, StatusCode::NOT_FOUND], 502)]
#[case::forbidden_after_missing([StatusCode::NOT_FOUND, StatusCode::NOT_FOUND, StatusCode::FORBIDDEN], 403)]
#[case::all_missing([StatusCode::NOT_FOUND, StatusCode::NOT_FOUND, StatusCode::NOT_FOUND], 404)]
#[tokio::test]
async fn card_resolver_discovery_failure_lists_every_attempt_in_order(
    #[case] statuses: [StatusCode; 3],
    #[case] expected_status: u16,
) {
    let [first, second, third] = statuses;
    let routes = vec![
        (AGENT_CARD, first, "first failure"),
        (AGENT_JSON, second, "second failure"),
        (FOUNDRY, third, "third failure"),
    ];
    let probed = probe(routes, None, &HeaderMap::new()).await;
    let error = probed.result.unwrap_err();
    assert_eq!(error.status_code(), expected_status);
    assert_eq!(paths(&probed.hits), vec![AGENT_CARD, AGENT_JSON, FOUNDRY]);
    let message = error.to_string();
    assert!(message.starts_with(&format!("no agent card found at {}. Tried ", probed.base)));
    let order: Vec<usize> = [AGENT_CARD, AGENT_JSON, FOUNDRY]
        .iter()
        .map(|path| message.find(path).unwrap())
        .collect();
    assert!(order.is_sorted());
}

#[tokio::test]
async fn card_resolver_unreachable_agent_lists_every_probe_without_a_status() {
    let closed = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let resolver = CardResolver {
        base_url: base.clone(),
        client: pooled_client(),
    };
    let error = resolver
        .get_agent_card(None, &HeaderMap::new())
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), 404);
    let message = error.to_string();
    assert!(message.contains(&format!("{base}. Tried {AGENT_CARD} (")));
    assert!(message.contains(AGENT_JSON));
    assert!(message.contains(FOUNDRY));
}

#[tokio::test]
async fn card_resolver_sends_extra_headers_with_every_probe() {
    let mut headers = HeaderMap::new();
    headers.insert("x-team", HeaderValue::from_static("search"));
    let probed = probe(Vec::new(), None, &headers).await;
    probed.result.unwrap_err();
    assert_eq!(
        probed.hits,
        vec![
            (AGENT_CARD.to_owned(), true),
            (AGENT_JSON.to_owned(), true),
            (FOUNDRY.to_owned(), true),
        ]
    );
}

#[rstest]
#[case::localhost("http://localhost:8000/", true)]
#[case::loopback_v4("http://127.0.0.1:9/", true)]
#[case::any_interface("http://0.0.0.0:8001/", true)]
#[case::ipv6_loopback("http://[::1]:8080/", true)]
#[case::upper_case("HTTP://LOCALHOST/", true)]
#[case::substring_in_path("https://example.com/localhost-docs", true)]
#[case::public_host("https://agent.example.com/", false)]
#[case::other_ipv6("http://[2001:db8::1]/", false)]
#[case::empty("", false)]
fn card_resolver_localhost_detection_matches_patterns(#[case] url: &str, #[case] expected: bool) {
    assert_eq!(is_localhost_or_internal_url(Some(url)), expected);
}

#[test]
fn card_resolver_missing_url_is_not_localhost() {
    assert!(!is_localhost_or_internal_url(None));
}

#[test]
fn card_resolver_localhost_patterns_are_litellm_patterns() {
    assert_eq!(
        LOCALHOST_URL_PATTERNS,
        ["localhost", "127.0.0.1", "0.0.0.0", "[::1]"]
    );
}

#[rstest]
#[case::jsonrpc_lowercase("jsonrpc", "JSONRPC", ProtocolVersion::V0_3)]
#[case::http_json_lowercase("http+json", "HTTP+JSON", ProtocolVersion::V0_3)]
#[case::grpc_mixed_case("Grpc", "GRPC", ProtocolVersion::V0_3)]
#[case::already_canonical("JSONRPC", "JSONRPC", ProtocolVersion::V1_0)]
#[case::unknown_binding("custom", "custom", ProtocolVersion::V1_0)]
fn card_resolver_normalizes_protocol_bindings(
    #[case] binding: &str,
    #[case] expected_binding: &str,
    #[case] expected_version: ProtocolVersion,
) {
    let card = card_with(&format!(
        r#"{{"supportedInterfaces":[{{"url":"https://agent.example/","protocolBinding":"{binding}","protocolVersion":"1.0"}}]}}"#
    ));
    let normalized = normalize_interfaces(card);
    let interface = first_interface(&normalized);
    assert_eq!(binding_of(interface), Some(expected_binding));
    assert_eq!(version_of(interface), Some(expected_version));
}

#[test]
fn card_resolver_leaves_non_string_binding_untouched() {
    let card =
        card_with(r#"{"supportedInterfaces":[{"protocolBinding":7,"protocolVersion":"1.0"}]}"#);
    let normalized = normalize_interfaces(card);
    let interface = first_interface(&normalized);
    assert_eq!(binding_of(interface), None);
    assert_eq!(version_of(interface), Some(ProtocolVersion::V1_0));
}

#[rstest]
#[case::top_level_wins(
    r#"{"url":"https://top.example/","supportedInterfaces":[{"url":"https://iface.example/"}]}"#,
    Some("https://top.example/")
)]
#[case::falls_back_to_interface(
    r#"{"supportedInterfaces":[{"url":"https://iface.example/"}]}"#,
    Some("https://iface.example/")
)]
#[case::empty_top_level_falls_back(
    r#"{"url":"","supportedInterfaces":[{"url":"https://iface.example/"}]}"#,
    Some("https://iface.example/")
)]
#[case::no_url(r#"{"name":"bare"}"#, None)]
fn card_resolver_reads_agent_card_url(#[case] fields: &str, #[case] expected: Option<&str>) {
    let card = card_with(fields);
    assert_eq!(get_agent_card_url(&card), expected);
}

#[rstest]
#[case::trims_trailing_slashes("https://agent.example///", "https://agent.example/")]
#[case::adds_trailing_slash("https://agent.example/v1", "https://agent.example/v1/")]
fn card_resolver_sets_normalized_agent_card_url(#[case] input: &str, #[case] expected: &str) {
    let mut card = card_with(r#"{"supportedInterfaces":[{"url":"https://old.example/"}]}"#);
    set_agent_card_url(&mut card, input);
    assert_eq!(get_agent_card_url(&card), Some(expected));
    assert!(first_interface(&card).url.is_some());
}

#[test]
fn card_resolver_set_url_rewrites_only_fields_the_card_carries() {
    let mut card = card_with(r#"{"supportedInterfaces":[{"url":"https://old.example/"}]}"#);
    set_agent_card_url(&mut card, "https://new.example");
    assert!(card.url.is_none());
    assert_eq!(get_agent_card_url(&card), Some("https://new.example/"));
}
