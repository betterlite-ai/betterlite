#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_auth::identity::{AgentIdentityFailure, FailureCode};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::jwt_settings::records::IssuerConfig;
use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;

use super::{IdentityStatusReads, agent_identity_routes};
use crate::agent_endpoints::identity_admin::issuers::IdentitySettings;
use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;

const FIRST_ISSUER: &str =
    "https://login.microsoftonline.com/00000000-0000-0000-0000-000000000001/v2.0";
const SECOND_ISSUER: &str =
    "https://login.microsoftonline.com/00000000-0000-0000-0000-0000000000ff/v2.0";
const BOUND: &str = r#"{"agent_id":"agent-1","agent_name":"Bound Agent","agent_card_params":{},"identity_managed":true,"execution_mode":"delegated","identity":{"agent_id":"agent-1","provider":"microsoft_entra","tenant_id":"00000000-0000-0000-0000-000000000001","client_id":"00000000-0000-0000-0000-000000000002","issuer":"https://login.microsoftonline.com/00000000-0000-0000-0000-000000000001/v2.0","revision":"rev-7","last_authenticated_at":"2026-01-02T03:04:05Z"}}"#;
const PLAIN: &str = r#"{"agent_id":"agent-1","agent_name":"Plain Agent","agent_card_params":{}}"#;
const DISABLED: &str = r#"{"agent_id":"agent-1","agent_name":"Disabled Agent","agent_card_params":{},"enabled":false}"#;
const BOTH: &str = r#"{"agent_id":"agent-1","agent_name":"Both Agent","agent_card_params":{},"identity_managed":true,"execution_mode":"both"}"#;
const UNREPRESENTABLE: &str = r#"{"agent_id":"agent-1","agent_name":"Bound Agent","agent_card_params":{},"identity_managed":true,"identity":{"agent_id":"agent-1","provider":"microsoft_entra","tenant_id":"tenant","client_id":"client","issuer":"issuer","revision":"rev-1"}}"#;
const PROVIDERS_PATH: &str = "/v1/agents/identity/providers";
const STATUS_PATH: &str = "/v1/agents/agent-1/identity";

fn entry(issuer: &str) -> IssuerConfig {
    serde_json::from_str(&format!(r#"{{"issuer":"{issuer}","audience":"api"}}"#)).unwrap()
}

fn settings(issuers: &[&str]) -> IdentitySettings {
    IdentitySettings {
        enable_jwt_auth: true,
        issuers: issuers.iter().copied().map(entry).collect(),
        jwt_issuer: None,
        jwt_audience: None,
        microsoft_tenant: None,
        microsoft_client_id: None,
    }
}

fn agent(fields: &str) -> AgentResponse {
    serde_json::from_str(fields).unwrap()
}

fn key(role: &str) -> UserAPIKeyAuth {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    fields.insert(String::from("user_role"), JsonData::String(role.to_owned()));
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

fn caller(role: &str) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: String::from("agent-admin"),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: Vec::new(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey(Some(key(role))),
        },
    }
}

fn keyless_caller() -> AuthContext {
    let mut context = caller("proxy_admin");
    context.principal.source_key = SourceKey(None);
    context
}

fn platform_admin_caller() -> AuthContext {
    let mut context = keyless_caller();
    context.principal.roles = vec![Role::PlatformAdmin];
    context
}

fn roleless_key_caller(roles: Vec<Role>) -> AuthContext {
    let mut context = caller("proxy_admin");
    context.principal.roles = roles;
    context.principal.source_key = SourceKey(Some(key_without_role()));
    context
}

fn key_without_role() -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).unwrap()
}

#[derive(Clone)]
struct FakeReads {
    agent: Option<AgentResponse>,
    failure: Option<AgentIdentityFailure>,
    reads: Arc<Mutex<Vec<String>>>,
}

impl FakeReads {
    fn answering(fields: &str) -> Self {
        Self {
            agent: Some(agent(fields)),
            failure: None,
            reads: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn failing(code: FailureCode, message: &str) -> Self {
        Self {
            agent: None,
            failure: Some(AgentIdentityFailure {
                code,
                message: message.to_owned(),
            }),
            reads: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn missing() -> Self {
        Self {
            agent: None,
            failure: None,
            reads: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn reads(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }
}

impl IdentityStatusReads for FakeReads {
    fn agent(
        &self,
        agent_id: &str,
    ) -> impl Future<Output = Result<Option<AgentResponse>, AgentIdentityFailure>> + Send {
        self.reads.lock().unwrap().push(agent_id.to_owned());
        if let Some(failure) = &self.failure {
            return ready(Err(failure.clone()));
        }
        ready(Ok(self
            .agent
            .as_ref()
            .filter(|agent| agent.agent_id == agent_id)
            .cloned()))
    }
}

struct Reply {
    status: StatusCode,
    body: Vec<u8>,
}

impl Reply {
    fn json<T>(&self) -> T
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.body).unwrap()
    }

    fn text(&self) -> String {
        String::from_utf8(self.body.clone()).unwrap()
    }
}

async fn call<Reads>(
    settings: IdentitySettings,
    reads: Reads,
    caller: Option<&AuthContext>,
    path: &str,
) -> Reply
where
    Reads: IdentityStatusReads + Clone + Send + Sync + 'static,
{
    let mut request = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    if let Some(context) = caller {
        request.extensions_mut().insert(context.clone());
    }
    let response = agent_identity_routes(settings, reads)
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    Reply {
        status,
        body: body.to_vec(),
    }
}

async fn get_providers(settings: IdentitySettings, caller: Option<&AuthContext>) -> Reply {
    call(settings, FakeReads::missing(), caller, PROVIDERS_PATH).await
}

async fn get_status(
    settings: IdentitySettings,
    reads: &FakeReads,
    caller: Option<&AuthContext>,
    path: &str,
) -> Reply {
    call(settings, reads.clone(), caller, path).await
}

#[tokio::test]
async fn providers_answers_the_trusted_issuers_for_an_admin() {
    let answer = get_providers(
        settings(&[FIRST_ISSUER, SECOND_ISSUER]),
        Some(&caller("proxy_admin")),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer.text(),
        format!(r#"["{FIRST_ISSUER}","{SECOND_ISSUER}"]"#)
    );
}

#[tokio::test]
async fn providers_answers_an_empty_list_without_jwt_auth() {
    let answer = get_providers(IdentitySettings::default(), Some(&caller("proxy_admin"))).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.text(), "[]");
}

#[tokio::test]
async fn providers_is_forbidden_for_a_non_admin() {
    let answer = get_providers(
        settings(&[FIRST_ISSUER]),
        Some(&caller("proxy_admin_viewer")),
    )
    .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    assert_eq!(
        answer.text(),
        r#"{"detail":{"error":"Only proxy admins can create, update, or delete agents. Your role=proxy_admin_viewer"}}"#
    );
}

#[tokio::test]
async fn providers_requires_an_authenticated_caller() {
    assert_eq!(
        get_providers(settings(&[FIRST_ISSUER]), None).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[rstest]
#[case::master_key(master_caller())]
#[case::platform_admin(platform_admin_caller())]
#[tokio::test]
async fn providers_answers_the_issuers_for_a_keyless_admin(#[case] given: AuthContext) {
    let answer = get_providers(settings(&[FIRST_ISSUER]), Some(&given)).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.text(), format!(r#"["{FIRST_ISSUER}"]"#));
}

#[tokio::test]
async fn providers_forbids_a_keyless_caller_without_a_role() {
    let answer = get_providers(settings(&[FIRST_ISSUER]), Some(&keyless_caller())).await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    assert_eq!(
        answer.text(),
        r#"{"detail":{"error":"Only proxy admins can create, update, or delete agents. Your role=internal_user"}}"#
    );
}

#[tokio::test]
async fn providers_forbids_a_keyless_viewer_with_its_role() {
    let mut viewer = keyless_caller();
    viewer.principal.roles = vec![Role::PlatformViewer];
    let answer = get_providers(settings(&[FIRST_ISSUER]), Some(&viewer)).await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    assert_eq!(
        answer.text(),
        r#"{"detail":{"error":"Only proxy admins can create, update, or delete agents. Your role=proxy_admin_viewer"}}"#
    );
}

#[tokio::test]
async fn providers_takes_a_roleless_keys_platform_role() {
    let answer = get_providers(
        settings(&[FIRST_ISSUER]),
        Some(&roleless_key_caller(vec![Role::PlatformAdmin])),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
}

#[tokio::test]
async fn providers_forbids_a_roleless_key_without_a_platform_role() {
    let answer = get_providers(
        settings(&[FIRST_ISSUER]),
        Some(&roleless_key_caller(Vec::new())),
    )
    .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    assert_eq!(
        answer.text(),
        r#"{"detail":{"error":"Only proxy admins can create, update, or delete agents. Your role=internal_user"}}"#
    );
}

#[rstest]
#[case::key(caller("proxy_admin"))]
#[case::platform_admin(platform_admin_caller())]
#[tokio::test]
async fn status_answers_the_identity_of_a_bound_agent(#[case] given: AuthContext) {
    let reads = FakeReads::answering(BOUND);
    let answer = get_status(settings(&[FIRST_ISSUER]), &reads, Some(&given), STATUS_PATH).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(reads.reads(), vec![String::from("agent-1")]);
    let reply: StatusReply = answer.json();
    assert_eq!(reply.execution_mode, "delegated");
    assert!(reply.identity_managed);
    assert!(reply.enabled);
    assert_eq!(
        reply.last_authenticated_at.as_deref(),
        Some("2026-01-02T03:04:05Z")
    );
    let binding = reply.identity.unwrap();
    assert_eq!(binding.agent_id, "agent-1");
    assert_eq!(binding.issuer, FIRST_ISSUER);
    assert_eq!(binding.revision, "rev-7");
}

#[tokio::test]
async fn status_answers_the_both_execution_mode() {
    let reads = FakeReads::answering(BOTH);
    let answer = get_status(
        settings(&[FIRST_ISSUER]),
        &reads,
        Some(&caller("proxy_admin")),
        STATUS_PATH,
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer.text(),
        r#"{"identity":null,"identity_managed":true,"enabled":true,"execution_mode":"both","last_authenticated_at":null}"#
    );
}

#[tokio::test]
async fn status_answers_a_plain_agent_with_nulls() {
    let reads = FakeReads::answering(PLAIN);
    let answer = get_status(
        settings(&[FIRST_ISSUER]),
        &reads,
        Some(&caller("proxy_admin")),
        STATUS_PATH,
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer.text(),
        r#"{"identity":null,"identity_managed":false,"enabled":true,"execution_mode":"autonomous","last_authenticated_at":null}"#
    );
}

#[tokio::test]
async fn status_reports_a_disabled_agent() {
    let reads = FakeReads::answering(DISABLED);
    let answer = get_status(
        settings(&[FIRST_ISSUER]),
        &reads,
        Some(&caller("proxy_admin")),
        STATUS_PATH,
    )
    .await;
    assert_eq!(
        answer.text(),
        r#"{"identity":null,"identity_managed":false,"enabled":false,"execution_mode":"autonomous","last_authenticated_at":null}"#
    );
}

#[tokio::test]
async fn status_answers_404_for_a_missing_agent() {
    let reads = FakeReads::missing();
    let answer = get_status(
        settings(&[FIRST_ISSUER]),
        &reads,
        Some(&caller("proxy_admin")),
        STATUS_PATH,
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert_eq!(answer.text(), r#"{"detail":"Agent not found"}"#);
}

#[tokio::test]
async fn status_answers_the_agent_named_by_the_path() {
    let reads = FakeReads::answering(BOUND);
    let answer = get_status(
        settings(&[FIRST_ISSUER]),
        &reads,
        Some(&caller("proxy_admin")),
        "/v1/agents/another-agent/identity",
    )
    .await;
    assert_eq!(reads.reads(), vec![String::from("another-agent")]);
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn status_answers_503_for_a_policy_unavailable_failure() {
    let reads = FakeReads::failing(
        FailureCode::PolicyUnavailable,
        "Agent policy could not be loaded",
    );
    let answer = get_status(
        settings(&[FIRST_ISSUER]),
        &reads,
        Some(&caller("proxy_admin")),
        STATUS_PATH,
    )
    .await;
    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        answer.text(),
        r#"{"detail":"Agent policy could not be loaded"}"#
    );
}

#[tokio::test]
async fn status_answers_403_for_a_denied_failure() {
    let reads = FakeReads::failing(FailureCode::IdentityDenied, "Agent identity was retracted");
    let answer = get_status(
        settings(&[FIRST_ISSUER]),
        &reads,
        Some(&caller("proxy_admin")),
        STATUS_PATH,
    )
    .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    assert_eq!(
        answer.text(),
        r#"{"detail":"Agent identity was retracted"}"#
    );
}

#[tokio::test]
async fn status_answers_503_for_a_binding_it_cannot_represent() {
    let reads = FakeReads::answering(UNREPRESENTABLE);
    let answer = get_status(
        settings(&[FIRST_ISSUER]),
        &reads,
        Some(&caller("proxy_admin")),
        STATUS_PATH,
    )
    .await;
    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        answer.text(),
        r#"{"detail":"Agent policy could not be loaded"}"#
    );
}

#[tokio::test]
async fn status_is_forbidden_for_a_non_admin_before_the_store_is_read() {
    let reads = FakeReads::answering(BOUND);
    let answer = get_status(
        settings(&[FIRST_ISSUER]),
        &reads,
        Some(&caller("proxy_admin_viewer")),
        STATUS_PATH,
    )
    .await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    assert_eq!(
        answer.text(),
        r#"{"detail":{"error":"Only proxy admins can create, update, or delete agents. Your role=proxy_admin_viewer"}}"#
    );
    assert!(reads.reads().is_empty());
}

#[tokio::test]
async fn status_requires_an_authenticated_caller() {
    let reads = FakeReads::answering(BOUND);
    assert_eq!(
        get_status(settings(&[FIRST_ISSUER]), &reads, None, STATUS_PATH)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert!(reads.reads().is_empty());
}

#[derive(Deserialize)]
struct StatusReply {
    identity: Option<BindingReply>,
    identity_managed: bool,
    enabled: bool,
    execution_mode: String,
    last_authenticated_at: Option<String>,
}

#[derive(Deserialize)]
struct BindingReply {
    agent_id: String,
    issuer: String,
    revision: String,
}
