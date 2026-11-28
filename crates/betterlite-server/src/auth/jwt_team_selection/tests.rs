#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_prefetch::permission::lookup::{
    ObjectPermissionCache, PermissionAnswer, PermissionCacheError,
};
use betterlite_auth::auth_team_lookup::lookup::TeamLookup;
use betterlite_auth::auth_team_lookup::ports::{TeamCache, TeamCacheError};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::jwt_claims::settings::JwtAuthSettings;
use betterlite_auth::jwt_team_selection::settings::SelectionSettings;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_paths::paths::Document;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{JwtTeamSelectionRule, JwtTeamSelectionStageInput, Selection, prefetch};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

const MODEL_ROUTE: &str = "/v1/files";

struct NoopCache;

#[async_trait]
impl TeamCache for NoopCache {
    async fn read_team(
        &self,
        _key: &str,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, TeamCacheError> {
        Ok(None)
    }

    async fn write_team(
        &self,
        _key: &str,
        _team: &LiteLLMTeamTableCachedObj,
        _ttl: f64,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }

    async fn cache_team(
        &self,
        _team_id: &str,
        _team: &mut LiteLLMTeamTableCachedObj,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }

    async fn read_access_group(
        &self,
        _key: &str,
    ) -> Result<Option<AccessGroupTable>, TeamCacheError> {
        Ok(None)
    }

    async fn cache_access_group(
        &self,
        _access_group_id: &str,
        _table: &AccessGroupTable,
    ) -> Result<(), TeamCacheError> {
        Ok(())
    }
}

#[async_trait]
impl ObjectPermissionCache for NoopCache {
    async fn read(&self, _key: &str) -> PermissionAnswer {
        Ok(None)
    }

    async fn write(
        &self,
        _key: &str,
        _permission: &LiteLLMObjectPermissionTable,
        _ttl: f64,
    ) -> Result<(), PermissionCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        0.0
    }
}

fn lookup() -> Arc<TeamLookup> {
    let cache: Arc<dyn TeamCache> = Arc::new(NoopCache);
    let permission_cache: Arc<dyn ObjectPermissionCache> = Arc::new(NoopCache);
    Arc::new(TeamLookup {
        cache,
        permission_cache,
        database: None,
        provisioner: None,
        cadence: Arc::new(Cadence::new(1024, Arc::new(WallClock))),
        in_memory_ttl: 0,
    })
}

fn rule(enforce_rbac: bool) -> Arc<JwtTeamSelectionRule> {
    Arc::new(JwtTeamSelectionRule {
        claims: serde_json::from_str("{}").expect("the claims decode"),
        claims_settings: JwtAuthSettings::default(),
        lookup: lookup(),
        request_headers: BTreeMap::new(),
        team_id_upsert: None,
        enforce_rbac,
        user_id: None,
        is_proxy_admin: false,
    })
}

fn config_with_required_team() -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(
        "general_settings:\n  master_key: secret-master-key\n  litellm_jwtauth:\n    team_id_jwt_field: team_id\n",
    )
    .expect("the test config loads")
}

fn master_context(master_key: &SecretString) -> AuthContext {
    let input = StageInput::MasterKey(Some(master_key.clone()));
    let Ok(context) = dispatch(AuthBackend::MasterKey, &input, master_key) else {
        panic!("the master key stage authenticates");
    };
    context
}

#[tokio::test]
async fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::JwtTeamSelection));

    let master_key = SecretString::from("secret-master-key".to_owned());
    let mut stage = JwtTeamSelectionStageInput {
        context: master_context(&master_key),
        rule: Some(rule(false)),
        settings: Some(SelectionSettings::default()),
        selection: Selection::default(),
    };
    prefetch(&mut stage).await.expect("the selection resolves");
    let input = StageInput::JwtTeamSelection(Box::new(stage));
    let Ok(forwarded) = dispatch(AuthBackend::JwtTeamSelection, &input, &master_key) else {
        panic!("the B732 constructor dispatches to the production selection stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::JwtTeamSelection, &wrong_stage, &master_key),
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

async fn run_request(
    config: &Config,
    rule: Arc<JwtTeamSelectionRule>,
) -> (StatusCode, String, bool) {
    let reached = Arc::new(AtomicBool::new(false));
    let application = routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(rule));
    let response = application
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

// a required team the token does not carry fails the stage before any downstream handler runs
#[tokio::test]
async fn failure_stops_downstream() {
    let (status, body, reached) = run_request(&config_with_required_team(), rule(false)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"401\""), "{body}");
    assert!(body.contains("No team found in token"), "{body}");
    assert!(!reached);
}

fn config_without_required_team() -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings:\n  master_key: secret-master-key\n")
    .expect("the test config loads")
}

// an enforced RBAC request with no user or team is refused after the claim selection step
#[tokio::test]
async fn object_identity_failure_stops_downstream() {
    let (status, body, reached) = run_request(&config_without_required_team(), rule(true)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"code\":\"403\""), "{body}");
    assert!(body.contains("enforce_rbac is set to True"), "{body}");
    assert!(!reached);
}
