use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use betterlite_auth::auth_identity::roles::Role;
use betterlite_core::agents::json::AgentJson;
use rstest::rstest;
use tower::ServiceExt as _;

use crate::api::a2a_discover::discovery::{AgentCardDiscoveryError, DiscoveryMode};
use crate::api::a2a_discover::{
    A2aDiscoverState, DISCOVER_PATH, DiscoverAgentCard, DiscoverAgentRequest, DiscoveryFailure,
    routes,
};

use super::support::auth_context;

const CARD: &str = r#"{"name":"agent","skills":[{"id":"echo"}]}"#;

fn card() -> AgentJson {
    serde_json::from_str(CARD).unwrap()
}

struct FakeDiscovery {
    outcome: Mutex<Option<Result<AgentJson, DiscoveryFailure>>>,
    requests: Mutex<Vec<DiscoverAgentRequest>>,
}

impl FakeDiscovery {
    fn answering(card: AgentJson) -> Arc<Self> {
        Self::with_outcome(Ok(card))
    }

    fn failing(failure: DiscoveryFailure) -> Arc<Self> {
        Self::with_outcome(Err(failure))
    }

    fn with_outcome(outcome: Result<AgentJson, DiscoveryFailure>) -> Arc<Self> {
        Arc::new(Self {
            outcome: Mutex::new(Some(outcome)),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> Vec<DiscoverAgentRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl DiscoverAgentCard for FakeDiscovery {
    async fn discover(
        &self,
        request: &DiscoverAgentRequest,
    ) -> Result<AgentJson, DiscoveryFailure> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        self.outcome
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| {
                Err(DiscoveryFailure::Unexpected {
                    message: "no scripted outcome".to_owned(),
                })
            })
    }
}

async fn call(
    discovery: Arc<FakeDiscovery>,
    body: &str,
    roles: &[Role],
) -> (StatusCode, AgentJson) {
    let request = Request::builder()
        .method("POST")
        .uri(DISCOVER_PATH)
        .header(header::CONTENT_TYPE, "application/json")
        .extension(auth_context(roles))
        .body(Body::from(body.to_owned()))
        .unwrap();
    let router: Router = routes(A2aDiscoverState { discovery });
    let response = router.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap();
    (status, value)
}

#[tokio::test]
async fn a_proxy_admin_gets_the_url_and_the_card() {
    let discovery = FakeDiscovery::answering(card());
    let (status, body) = call(
        Arc::clone(&discovery),
        r#"{"url":"https://agent.test/"}"#,
        &[Role::PlatformAdmin],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["url"],
        AgentJson::String("https://agent.test/".to_owned())
    );
    assert_eq!(body["agent_card"], card());
    let requests = discovery.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "https://agent.test/");
    assert_eq!(requests[0].discovery_mode, DiscoveryMode::WellKnownFallback);
    assert!(requests[0].params.is_empty());
}

#[rstest]
#[case::platform_viewer(&[Role::PlatformViewer], "platform_viewer")]
#[case::org_admin(&[Role::OrgAdmin], "org_admin")]
#[case::team_admin(&[Role::TeamAdmin], "team_admin")]
#[case::no_roles(&[], "internal_user")]
#[tokio::test]
async fn a_non_admin_is_forbidden_with_its_role(#[case] roles: &[Role], #[case] expected: &str) {
    let discovery = FakeDiscovery::answering(card());
    let (status, body) = call(
        Arc::clone(&discovery),
        r#"{"url":"https://agent.test"}"#,
        roles,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body["detail"],
        AgentJson::String(format!(
            "Only proxy admins can discover agent cards. Your role={expected}"
        ))
    );
    assert!(discovery.requests().is_empty());
}

#[tokio::test]
async fn a_discovery_error_is_a_bad_request() {
    let discovery = FakeDiscovery::failing(DiscoveryFailure::Card(
        AgentCardDiscoveryError::MissingAssistantId,
    ));
    let (status, body) = call(
        discovery,
        r#"{"url":"https://agent.test","discovery_mode":"langgraph_platform"}"#,
        &[Role::PlatformAdmin],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["detail"],
        AgentJson::String("langgraph_platform discovery requires params.assistant_id".to_owned())
    );
}

#[tokio::test]
async fn an_unexpected_error_is_an_internal_error() {
    let discovery = FakeDiscovery::failing(DiscoveryFailure::Unexpected {
        message: "boom".to_owned(),
    });
    let (status, body) = call(
        discovery,
        r#"{"url":"https://agent.test"}"#,
        &[Role::PlatformAdmin],
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body["detail"],
        AgentJson::String("Discovery failed: boom".to_owned())
    );
}

#[tokio::test]
async fn a_body_that_is_not_json_is_a_bad_request() {
    let discovery = FakeDiscovery::answering(card());
    let (status, body) = call(discovery, "not json", &[Role::PlatformAdmin]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .starts_with("invalid request body:")
    );
}

#[tokio::test]
async fn a_body_without_a_url_is_a_bad_request() {
    let discovery = FakeDiscovery::answering(card());
    let (status, body) = call(discovery, "{}", &[Role::PlatformAdmin]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["detail"].as_str().unwrap().contains("url"));
}

#[tokio::test]
async fn a_url_of_the_wrong_type_is_a_bad_request_that_names_it() {
    let discovery = FakeDiscovery::answering(card());
    let (status, body) = call(discovery, r#"{"url":5}"#, &[Role::PlatformAdmin]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let detail = body["detail"].as_str().unwrap();
    assert!(detail.contains("url"), "{detail}");
    assert!(detail.contains("expected a string"), "{detail}");
}

#[tokio::test]
async fn an_unknown_discovery_mode_is_a_bad_request() {
    let discovery = FakeDiscovery::answering(card());
    let (status, body) = call(
        discovery,
        r#"{"url":"https://agent.test","discovery_mode":"bogus"}"#,
        &[Role::PlatformAdmin],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let detail = body["detail"].as_str().unwrap();
    assert!(detail.contains("discovery_mode"), "{detail}");
    assert!(detail.contains("well_known_fallback"), "{detail}");
}

#[tokio::test]
async fn an_explicit_null_discovery_mode_takes_the_default() {
    let discovery = FakeDiscovery::answering(card());
    let (status, _body) = call(
        Arc::clone(&discovery),
        r#"{"url":"https://agent.test","discovery_mode":null}"#,
        &[Role::PlatformAdmin],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        discovery.requests()[0].discovery_mode,
        DiscoveryMode::WellKnownFallback
    );
}

#[tokio::test]
async fn an_explicit_null_params_becomes_an_empty_map() {
    let discovery = FakeDiscovery::answering(card());
    let (status, _body) = call(
        Arc::clone(&discovery),
        r#"{"url":"https://agent.test","params":null}"#,
        &[Role::PlatformAdmin],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(discovery.requests()[0].params.is_empty());
}

#[tokio::test]
async fn langgraph_platform_reaches_the_discovery_with_its_params() {
    let discovery = FakeDiscovery::answering(card());
    let (status, _body) = call(
        Arc::clone(&discovery),
        r#"{"url":"https://agent.test","discovery_mode":"langgraph_platform","params":{"assistant_id":"a1","extra":true}}"#,
        &[Role::PlatformAdmin],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let request = &discovery.requests()[0];
    assert_eq!(request.discovery_mode, DiscoveryMode::LanggraphPlatform);
    assert_eq!(
        request.params.get("assistant_id"),
        Some(&AgentJson::String("a1".to_owned()))
    );
}

#[tokio::test]
async fn trailing_data_after_the_json_body_is_a_bad_request() {
    let discovery = FakeDiscovery::answering(card());
    let (status, body) = call(
        discovery,
        r#"{"url":"https://agent.test"} trailing"#,
        &[Role::PlatformAdmin],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["detail"]
            .as_str()
            .unwrap()
            .starts_with("invalid request body:")
    );
}

#[tokio::test]
async fn a_params_value_of_the_wrong_type_is_a_bad_request_that_names_it() {
    let discovery = FakeDiscovery::answering(card());
    let (status, body) = call(
        discovery,
        r#"{"url":"https://agent.test","params":[1]}"#,
        &[Role::PlatformAdmin],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let detail = body["detail"].as_str().unwrap();
    assert!(detail.contains("params"), "{detail}");
}
