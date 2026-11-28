#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_router::adaptive_classification::config::AdaptiveRouterWeights;
use betterlite_router::adaptive_classification::request_type::RequestType;
use betterlite_router::adaptive_selection::{SnapshotCell, StateSnapshot};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::{caller_through_key_stage, master_caller};

use super::{
    ADAPTIVE_ROUTER_STATE_PATH, AdaptiveRouterSnapshots, HOME_PATH, RootRouteState, routes,
};

const ADMIN_ONLY_DETAIL: &str = "Admin-only endpoint. Not allowed to access this.";
const NO_ROUTER_DETAIL: &str = "No adaptive_router is configured on this proxy.";

#[tokio::test]
async fn home_answers_the_running_string_to_an_authenticated_caller() {
    let (root_state, fake_routers) = route_state(Vec::new());
    let reply = get(root_state, Some(&caller("key-owner", &[])), HOME_PATH).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.content_type.as_deref(), Some("application/json"));
    assert_eq!(reply.json::<String>(), "LiteLLM: RUNNING");
    assert_eq!(fake_routers.reads(), 0);
}

#[rstest]
#[case::home(HOME_PATH)]
#[case::adaptive_router_state(ADAPTIVE_ROUTER_STATE_PATH)]
#[tokio::test]
async fn the_keyed_routes_take_only_get(#[case] path: &str) {
    let (root_state, _fake_routers) = route_state(Vec::new());
    let reply = call(root_state, None, "POST", path).await;
    assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[rstest]
#[case::proxy_admin_key(key_caller("proxy_admin"))]
#[case::proxy_admin_viewer_key(key_caller("proxy_admin_viewer"))]
#[case::master_key(master_caller())]
#[case::platform_admin_role(caller("jwt-user", &[Role::PlatformAdmin]))]
#[case::platform_viewer_role(caller("jwt-user", &[Role::PlatformViewer]))]
#[tokio::test]
async fn state_admits_every_admin_view_caller(#[case] caller_context: AuthContext) {
    let (root_state, fake_routers) = route_state(vec![Vec::new()]);
    let reply = get(
        root_state,
        Some(&caller_context),
        ADAPTIVE_ROUTER_STATE_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(fake_routers.reads(), 1);
}

#[rstest]
#[case::internal_user_key(key_caller("internal_user"))]
#[case::internal_user_viewer_key(key_caller("internal_user_viewer"))]
#[case::org_admin_key(key_caller("org_admin"))]
#[case::team_key(key_caller("team"))]
#[case::customer_key(key_caller("customer"))]
#[case::org_admin_role(caller("role-user", &[Role::OrgAdmin]))]
#[case::no_role(caller("plain-user", &[]))]
#[tokio::test]
async fn state_refuses_every_other_caller_before_reading_the_router(
    #[case] caller_context: AuthContext,
) {
    let (root_state, fake_routers) = route_state(vec![vec![snapshot("bandit", &["fast"])]]);
    let reply = get(
        root_state,
        Some(&caller_context),
        ADAPTIVE_ROUTER_STATE_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.json::<DetailReply>().detail.error, ADMIN_ONLY_DETAIL);
    assert_eq!(fake_routers.reads(), 0);
}

#[tokio::test]
async fn a_key_whose_user_is_master_key_gets_the_admin_view_error() {
    let (root_state, fake_routers) = route_state(vec![vec![snapshot("bandit", &["fast"])]]);
    let reply = get(
        root_state,
        Some(&caller_through_key_stage(r#"{"user_id":"master_key"}"#)),
        ADAPTIVE_ROUTER_STATE_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.json::<DetailReply>().detail.error, ADMIN_ONLY_DETAIL);
    assert_eq!(fake_routers.reads(), 0);
}

#[tokio::test]
async fn state_refuses_a_request_without_an_authenticated_caller() {
    let (root_state, fake_routers) = route_state(vec![vec![snapshot("bandit", &["fast"])]]);
    let reply = get(root_state, None, ADAPTIVE_ROUTER_STATE_PATH).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.json::<DetailReply>().detail.error, ADMIN_ONLY_DETAIL);
    assert_eq!(fake_routers.reads(), 0);
}

#[tokio::test]
async fn state_answers_not_configured_without_any_adaptive_router() {
    let (root_state, fake_routers) = route_state(Vec::new());
    let reply = get(
        root_state,
        Some(&master_caller()),
        ADAPTIVE_ROUTER_STATE_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.json::<DetailReply>().detail.error, NO_ROUTER_DETAIL);
    assert_eq!(fake_routers.reads(), 1);
}

#[tokio::test]
async fn state_answers_an_empty_list_when_no_deployment_is_registered() {
    let (root_state, _fake_routers) = route_state(vec![Vec::new(), Vec::new()]);
    let reply = get(
        root_state,
        Some(&master_caller()),
        ADAPTIVE_ROUTER_STATE_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.json::<RoutersReply>().routers.is_empty());
}

#[tokio::test]
async fn state_lists_snapshots_in_registry_order() {
    let (root_state, _fake_routers) = route_state(vec![
        vec![
            snapshot("alpha-second", &["slow"]),
            snapshot("alpha-first", &["fast"]),
        ],
        vec![snapshot("beta", &["medium"])],
    ]);
    let reply = get(
        root_state,
        Some(&master_caller()),
        ADAPTIVE_ROUTER_STATE_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: RoutersReply = reply.json();
    let names: Vec<&str> = body
        .routers
        .iter()
        .map(|router| router.router_name.as_str())
        .collect();
    assert_eq!(names, ["alpha-second", "alpha-first", "beta"]);
}

#[tokio::test]
async fn state_serializes_every_snapshot_field() {
    let (root_state, _fake_routers) = route_state(vec![vec![detailed_snapshot()]]);
    let reply = get(
        root_state,
        Some(&master_caller()),
        ADAPTIVE_ROUTER_STATE_PATH,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: RoutersReply = reply.json();
    let router = body.routers.first().expect("one adaptive router");
    assert_eq!(router.router_name, "bandit");
    assert_eq!(
        router.available_models,
        ["fast".to_owned(), "slow".to_owned()]
    );
    assert_eq!(router.weights.quality.to_bits(), 0.75_f64.to_bits());
    assert_eq!(router.weights.economy.to_bits(), 0.25_f64.to_bits());
    let costs: Vec<(&str, Decimal)> = router
        .model_costs
        .iter()
        .map(|(model, amount)| (model.as_str(), *amount))
        .collect();
    assert_eq!(
        costs,
        [("fast", Decimal::new(3, 6)), ("slow", Decimal::new(15, 6))]
    );
    assert_eq!(router.cells.len(), 2);
    let first = &router.cells[0];
    assert_eq!(first.request_type, "code_generation");
    assert_eq!(first.model, "fast");
    assert_eq!(first.alpha.to_bits(), 3.5_f64.to_bits());
    assert_eq!(first.beta.to_bits(), 1.5_f64.to_bits());
    assert_eq!(first.samples, 5);
    assert_eq!(first.quality_mean.to_bits(), 0.7_f64.to_bits());
    let second = &router.cells[1];
    assert_eq!(second.request_type, "writing");
    assert_eq!(second.model, "slow");
    assert_eq!(second.samples, 4);
}

#[tokio::test]
async fn state_serializes_model_costs_as_json_numbers() {
    let (root_state, _fake_routers) = route_state(vec![vec![detailed_snapshot()]]);
    let reply = get(
        root_state,
        Some(&master_caller()),
        ADAPTIVE_ROUTER_STATE_PATH,
    )
    .await;
    let raw = String::from_utf8(reply.body).expect("the body is UTF-8");
    assert!(raw.contains(r#""model_costs":{"fast":0.000003,"slow":0.000015}"#));
}

fn snapshot(router_name: &str, models: &[&str]) -> StateSnapshot {
    StateSnapshot {
        router_name: router_name.to_owned(),
        available_models: models.iter().map(|model| (*model).to_owned()).collect(),
        weights: AdaptiveRouterWeights {
            quality: 0.5,
            economy: 0.5,
        },
        model_costs: Vec::new(),
        cells: Vec::new(),
    }
}

fn detailed_snapshot() -> StateSnapshot {
    StateSnapshot {
        router_name: "bandit".to_owned(),
        available_models: vec!["fast".to_owned(), "slow".to_owned()],
        weights: AdaptiveRouterWeights {
            quality: 0.75,
            economy: 0.25,
        },
        model_costs: vec![
            ("fast".to_owned(), Decimal::new(3, 6)),
            ("slow".to_owned(), Decimal::new(15, 6)),
        ],
        cells: vec![
            SnapshotCell {
                request_type: RequestType::CodeGeneration,
                model: "fast".to_owned(),
                alpha: 3.5,
                beta: 1.5,
                samples: 5,
                quality_mean: 0.7,
            },
            SnapshotCell {
                request_type: RequestType::Writing,
                model: "slow".to_owned(),
                alpha: 2.0,
                beta: 2.0,
                samples: 4,
                quality_mean: 0.5,
            },
        ],
    }
}

struct FakeAdaptiveRouters {
    groups: Vec<Vec<StateSnapshot>>,
    reads: AtomicUsize,
}

impl FakeAdaptiveRouters {
    fn answering(groups: Vec<Vec<StateSnapshot>>) -> Arc<Self> {
        Arc::new(Self {
            groups,
            reads: AtomicUsize::new(0),
        })
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl AdaptiveRouterSnapshots for FakeAdaptiveRouters {
    fn snapshots(&self) -> Vec<Vec<StateSnapshot>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.groups.clone()
    }
}

fn route_state(groups: Vec<Vec<StateSnapshot>>) -> (RootRouteState, Arc<FakeAdaptiveRouters>) {
    let fake_routers = FakeAdaptiveRouters::answering(groups);
    let shared: Arc<FakeAdaptiveRouters> = Arc::clone(&fake_routers);
    let adaptive_routers: Arc<dyn AdaptiveRouterSnapshots> = shared;
    (RootRouteState { adaptive_routers }, fake_routers)
}

fn caller(subject: &str, roles: &[Role]) -> AuthContext {
    context(subject, roles, SourceKey::default())
}

fn key_caller(role: &str) -> AuthContext {
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_str(&format!(r#"{{"user_role": "{role}"}}"#))
            .expect("the key fixture parses");
    let key =
        UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the key fixture constructs");
    context("key-owner", &[], SourceKey(Some(key)))
}

fn context(subject: &str, roles: &[Role], source_key: SourceKey) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: roles.to_vec(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key,
        },
    }
}

struct Reply {
    status: StatusCode,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Reply {
    fn json<T>(&self) -> T
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.body).expect("json body")
    }
}

async fn get(
    root_state: RootRouteState,
    caller_context: Option<&AuthContext>,
    path: &str,
) -> Reply {
    call(root_state, caller_context, "GET", path).await
}

async fn call(
    root_state: RootRouteState,
    caller_context: Option<&AuthContext>,
    method: &str,
    path: &str,
) -> Reply {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .expect("request");
    if let Some(context) = caller_context {
        request.extensions_mut().insert(context.clone());
    }
    let response = routes(root_state)
        .oneshot(request)
        .await
        .expect("router response");
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    Reply {
        status,
        content_type,
        body: body.to_vec(),
    }
}

#[derive(Deserialize)]
struct RoutersReply {
    routers: Vec<SnapshotReply>,
}

#[derive(Deserialize)]
struct SnapshotReply {
    router_name: String,
    available_models: Vec<String>,
    weights: WeightsReply,
    model_costs: IndexMap<String, Decimal>,
    cells: Vec<CellReply>,
}

#[derive(Deserialize)]
struct WeightsReply {
    quality: f64,
    #[serde(rename = "cost")]
    economy: f64,
}

#[derive(Deserialize)]
struct CellReply {
    request_type: String,
    model: String,
    alpha: f64,
    beta: f64,
    samples: u64,
    quality_mean: f64,
}

#[derive(Deserialize)]
struct DetailReply {
    detail: DetailErrorReply,
}

#[derive(Deserialize)]
struct DetailErrorReply {
    error: String,
}
