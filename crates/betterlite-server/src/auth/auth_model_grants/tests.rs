#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::agent_access_groups::ceiling::AgentAccessGroupCeiling;
use betterlite_auth::agent_access_groups::error::AccessGroupsError;
use betterlite_auth::agent_caller::error::AgentCallerError;
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::auth_model_grants::access_source::AgentAccessSource;
use betterlite_auth::auth_model_patterns::denial::model_denied_client_message;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::auth_team_lookup::error::{TeamFailure, TeamLookupError};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{AgentModelGrantsRule, stage_failure, stage_input};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::auth::prefetch_stage;
use crate::auth::team_budget_admission::TeamBudgetRule;
use crate::routes;
use crate::routes::tests::state_for;
use crate::state::ServerState;

// a recognized route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";
const AGENT_KEY: &str = r#"{"token":"sk-agent","agent_id":"agent-1"}"#;

fn config_with_master_key() -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings:\n  master_key: secret-master-key\n")
    .expect("the test config loads")
}

fn master_context(master_key: &SecretString) -> AuthContext {
    let input = StageInput::MasterKey(Some(master_key.clone()));
    let Ok(context) = dispatch(AuthBackend::MasterKey, &input, master_key) else {
        panic!("the master key stage authenticates");
    };
    context
}

fn key() -> UserAPIKeyAuth {
    let decoded: BTreeMap<String, JsonData> =
        serde_json::from_str(AGENT_KEY).expect("the key fields decode");
    UserAPIKeyAuth::construct(AuthInput::Dict(decoded)).expect("the key fixture constructs")
}

fn rule(model: &str) -> Arc<AgentModelGrantsRule> {
    Arc::new(AgentModelGrantsRule {
        key: Some(key()),
        model: RequestedModels::One(model.to_owned()),
        view: None,
        provider: None,
        global_model_aliases: BTreeMap::new(),
        teams: None,
        member_checks: Arc::new(TeamBudgetRule::default()),
    })
}

fn ceiling(models: &[&str]) -> AgentAccessGroupCeiling {
    AgentAccessGroupCeiling {
        access_group_ids: vec!["group-1".to_owned()],
        models: models.iter().map(|name| (*name).to_owned()).collect(),
        mcp_server_ids: BTreeSet::new(),
        agent_ids: BTreeSet::new(),
    }
}

// the installed source answers only the unmanaged ceiling, which is all these requests read
struct Unmanaged {
    models: Vec<&'static str>,
}

#[async_trait]
impl AgentAccessSource for Unmanaged {
    fn managed_policy(&self, _auth: Option<&UserAPIKeyAuth>) -> Option<AgentResponse> {
        None
    }

    async fn managed_ceilings(
        &self,
        _policy: &AgentResponse,
    ) -> Result<Vec<AgentAccessGroupCeiling>, AccessGroupsError> {
        Ok(Vec::new())
    }

    async fn unmanaged_ceiling(
        &self,
        _agent_id: &str,
    ) -> Result<Option<AgentAccessGroupCeiling>, AccessGroupsError> {
        Ok(Some(ceiling(&self.models)))
    }

    async fn caller_auth(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<Option<UserAPIKeyAuth>, AgentCallerError> {
        Ok(None)
    }

    async fn caller_team(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, AgentCallerError> {
        Ok(None)
    }

    async fn caller_user(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<Option<LiteLLMUserTable>, AgentCallerError> {
        Ok(None)
    }
}

fn state_with_source(config: &Config, source: Arc<dyn AgentAccessSource>) -> Arc<ServerState> {
    let mut state = state_for(config);
    Arc::get_mut(&mut state)
        .expect("the state is unique")
        .auth_model_grants
        .source = Some(source);
    state
}

fn staged_request(
    config: &Config,
    rule: &Arc<AgentModelGrantsRule>,
    source: Arc<dyn AgentAccessSource>,
    context: &AuthContext,
) -> StageInput {
    let mut request = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .body(Body::empty())
        .expect("the request builds");
    request.extensions_mut().insert(Arc::clone(rule));
    request
        .extensions_mut()
        .insert(state_with_source(config, source));
    let Ok(staged) = stage_input(&request, Some(context)) else {
        panic!("the B732 constructor builds the stage input");
    };
    staged
}

#[tokio::test]
async fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthModelGrants));

    let master_key = SecretString::from("secret-master-key".to_owned());
    let context = master_context(&master_key);
    let config = config_with_master_key();

    let allowed = staged_request(
        &config,
        &rule("claude"),
        Arc::new(Unmanaged {
            models: vec!["claude"],
        }),
        &context,
    );
    let Ok(StageInput::AuthModelGrants(stage)) = prefetch_stage(allowed).await else {
        panic!("the installed source forwards the agent grant stage");
    };
    let Ok(forwarded) = dispatch(
        AuthBackend::AuthModelGrants,
        &StageInput::AuthModelGrants(stage),
        &master_key,
    ) else {
        panic!("the B732 constructor dispatches to the production agent grant stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let refused = staged_request(
        &config,
        &rule("claude"),
        Arc::new(Unmanaged { models: vec![] }),
        &context,
    );
    assert!(matches!(
        prefetch_stage(refused).await,
        Err(StageFailure::ModelAccess(_))
    ));
}

#[tokio::test]
async fn missing_source_is_initialization_error() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let context = master_context(&master_key);
    let config = config_with_master_key();
    // a configured agent key without the installed source is an initialization error, not an empty ceiling
    let mut request = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .body(Body::empty())
        .expect("the request builds");
    request.extensions_mut().insert(rule("claude"));
    request.extensions_mut().insert(state_for(&config));
    let Ok(staged) = stage_input(&request, Some(&context)) else {
        panic!("the stage input builds");
    };
    assert!(matches!(
        prefetch_stage(staged).await,
        Err(StageFailure::ModelGrants(_))
    ));
}

#[tokio::test]
async fn wrong_stage_is_authentication_failure() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::AuthModelGrants, &wrong_stage, &master_key),
        Err(StageFailure::Authentication(_))
    ));
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

fn request_to(reached: &Arc<AtomicBool>) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri(MODEL_ROUTE)
        .header(header::AUTHORIZATION, "Bearer secret-master-key")
        .body(Body::empty())
        .expect("the request builds");
    request.extensions_mut().insert(Arc::clone(reached));
    request
}

fn application(
    config: &Config,
    rule: Arc<AgentModelGrantsRule>,
    source: Arc<dyn AgentAccessSource>,
) -> Router {
    routes::router(
        state_with_source(config, source),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(rule))
}

async fn run_request(
    config: &Config,
    rule: Arc<AgentModelGrantsRule>,
    source: Arc<dyn AgentAccessSource>,
) -> (StatusCode, String, bool) {
    let reached = Arc::new(AtomicBool::new(false));
    let response = application(config, rule, source)
        .oneshot(request_to(&reached))
        .await
        .expect("the request reaches the router");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    let text = String::from_utf8(body.to_vec()).expect("the response body is UTF-8");
    (status, text, reached.load(Ordering::Acquire))
}

#[tokio::test]
async fn failure_stops_downstream() {
    let (status, body, reached) = run_request(
        &config_with_master_key(),
        rule("claude"),
        Arc::new(Unmanaged { models: vec![] }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body.contains("\"type\":\"agent_model_access_denied\""),
        "{body}"
    );
    assert!(body.contains("\"param\":\"model\""), "{body}");
    assert!(body.contains("\"code\":\"403\""), "{body}");
    assert!(
        body.contains(model_denied_client_message("claude").as_str()),
        "{body}"
    );
    // the private diagnostic names the agent's groups and stays out of the response
    assert!(!body.contains("access groups grant no models"), "{body}");
    assert!(!reached);

    let (allowed_status, allowed_body, allowed_reached) = run_request(
        &config_with_master_key(),
        rule("claude"),
        Arc::new(Unmanaged {
            models: vec!["claude"],
        }),
    )
    .await;
    assert_eq!(allowed_status, StatusCode::OK, "{allowed_body}");
    assert!(allowed_reached);
}

#[test]
fn lookup_http_errors_keep_their_status() {
    let missing_group = AccessGroupsError::Read {
        access_group_id: "group-1".to_owned(),
        source: Box::new(TeamLookupError::AccessGroupNotFound {
            access_group_id: "group-1".to_owned(),
        }),
    };
    let StageFailure::ModelGrants(missing_group) = stage_failure(
        betterlite_auth::auth_model_grants::error::AgentModelGrantsError::Ceiling(Box::new(
            missing_group,
        )),
    ) else {
        panic!("missing access groups preserve their HTTP status");
    };
    assert_eq!(missing_group.code, "404");
    assert_eq!(missing_group.error_type, "auth_error");
    assert_eq!(missing_group.param.as_deref(), Some("None"));

    let unavailable = AccessGroupsError::Read {
        access_group_id: "group-1".to_owned(),
        source: Box::new(TeamLookupError::AccessPolicyUnavailable {
            access_group_id: "group-1".to_owned(),
            cause: Box::new(TeamFailure::Throttled),
        }),
    };
    let StageFailure::ModelGrants(unavailable) = stage_failure(
        betterlite_auth::auth_model_grants::error::AgentModelGrantsError::Ceiling(Box::new(
            unavailable,
        )),
    ) else {
        panic!("unavailable access policies preserve their HTTP status");
    };
    assert_eq!(unavailable.code, "503");

    let caller = AgentCallerError::Team {
        team_id: "missing-team".to_owned(),
        source: Box::new(TeamLookupError::TeamNotFound {
            team_id: "missing-team".to_owned(),
        }),
    };
    let StageFailure::ModelGrants(caller) = stage_failure(
        betterlite_auth::auth_model_grants::error::AgentModelGrantsError::Caller(Box::new(caller)),
    ) else {
        panic!("missing caller teams preserve their HTTP status");
    };
    assert_eq!(caller.code, "404");
}

#[test]
fn debug_reports_the_key_without_it() {
    let grants = rule("claude");
    let rendered = format!("{grants:?}");
    assert!(rendered.contains("has_key: true"), "{rendered}");
    // the token is never rendered, so a log line cannot carry the key
    assert!(!rendered.contains("sk-agent"), "{rendered}");
}

#[test]
fn debug_reports_the_wired_source() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let context = master_context(&master_key);
    let config = config_with_master_key();
    let staged = staged_request(
        &config,
        &rule("claude"),
        Arc::new(Unmanaged {
            models: vec!["claude"],
        }),
        &context,
    );
    let StageInput::AuthModelGrants(stage) = staged else {
        panic!("the agent grant stage input builds");
    };
    let rendered = format!("{stage:?}");
    assert!(rendered.contains("has_source: true"), "{rendered}");
    assert!(!rendered.contains("sk-agent"), "{rendered}");
}
