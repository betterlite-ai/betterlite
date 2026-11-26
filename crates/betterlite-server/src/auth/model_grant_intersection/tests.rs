#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_org_lookup::error::OrgLookupError;
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_team_lookup::error::TeamLookupError;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::end_user_registry::errors::RegistryError;
use betterlite_auth::membership_load::error::MembershipLoadError;
use betterlite_auth::model_grant_intersection::sources::{
    GrantSources, ServingFailure, ServingGroups,
};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;

use super::{GrantRule, GrantSink, GrantStageInput, now_seconds};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

// a recognized route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TeamAnswer {
    Row,
    Missing,
}

struct ScriptedSources {
    team: TeamAnswer,
}

#[async_trait]
impl GrantSources for ScriptedSources {
    async fn team_object(
        &self,
        team_id: &str,
    ) -> Result<LiteLLMTeamTableCachedObj, TeamLookupError> {
        match self.team {
            TeamAnswer::Row => Ok(team_row("tier-a")),
            TeamAnswer::Missing => Err(TeamLookupError::TeamNotFound {
                team_id: team_id.to_owned(),
            }),
        }
    }

    async fn user_object(
        &self,
        _user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, UserLookupError> {
        Ok(None)
    }

    async fn project_object(
        &self,
        _project_id: &str,
        _now: f64,
    ) -> Result<Option<CachedProject>, OrgLookupError> {
        Ok(None)
    }

    async fn organization(
        &self,
        _org_id: &str,
    ) -> Result<Option<OrganizationTable>, OrgLookupError> {
        Ok(None)
    }

    async fn membership(
        &self,
        _user_id: &str,
        _team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        Ok(None)
    }

    async fn budget_groups(&self) -> Result<Option<BTreeSet<String>>, RegistryError> {
        Ok(None)
    }
}

struct ScriptedGroups;

#[async_trait]
impl ServingGroups for ScriptedGroups {
    async fn groups_for(
        &self,
        model: &str,
        _team_id: Option<&str>,
    ) -> Result<BTreeSet<String>, ServingFailure> {
        let mut groups = BTreeSet::new();
        if model == "gpt-4o" {
            groups.insert("tier-a".to_owned());
        }
        Ok(groups)
    }
}

#[derive(Default)]
struct Recorder {
    allowlists: Mutex<Option<Vec<Vec<String>>>>,
    matched: Mutex<Option<Vec<String>>>,
}

impl Recorder {
    fn recorded(&self) -> Option<(Vec<Vec<String>>, Vec<String>)> {
        let allowlists = self
            .allowlists
            .lock()
            .expect("the recorded allowlists")
            .clone()?;
        let matched = self
            .matched
            .lock()
            .expect("the recorded groups")
            .clone()
            .unwrap_or_default();
        Some((allowlists, matched))
    }
}

impl GrantSink for Recorder {
    fn record(&self, allowlists: &[Vec<String>], matched: &[String]) {
        *self.allowlists.lock().expect("the recorded allowlists") = Some(allowlists.to_vec());
        *self.matched.lock().expect("the recorded groups") = Some(matched.to_vec());
    }
}

fn row<Row: DeserializeOwned>(text: &str) -> Row {
    serde_json::from_str(text).expect("the row fixture parses")
}

fn team_row(models: &str) -> LiteLLMTeamTableCachedObj {
    row(&format!(
        r#"{{"team_id":"team-1","models":["{models}"],"last_refreshed_at":1.0}}"#
    ))
}

fn key() -> UserAPIKeyAuth {
    let mut fields = BTreeMap::new();
    fields.insert(
        "models".to_owned(),
        JsonData::Array(vec![JsonData::String("tier-a".to_owned())]),
    );
    fields.insert("team_id".to_owned(), JsonData::String("team-1".to_owned()));
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the key fixture builds")
}

fn rule(sources: Option<Arc<ScriptedSources>>, recorder: Arc<Recorder>) -> Arc<GrantRule> {
    Arc::new(GrantRule {
        key: Some(key()),
        team: None,
        project: None,
        membership: None,
        membership_loaded: false,
        models: Some(vec!["gpt-4o".to_owned()]),
        sources,
        groups: Some(Arc::new(ScriptedGroups)),
        sink: Some(recorder),
    })
}

fn config(environment: &[(&str, &str)], yaml: &str) -> Config {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(yaml)
        .expect("the test config loads")
}

fn config_with_master_key() -> Config {
    config(&[], "general_settings:\n  master_key: secret-master-key\n")
}

const DATABASE_URL: &str = "postgresql://user:password@localhost:5432/betterlite";

fn config_with_database() -> Config {
    config(
        &[("DATABASE_URL", DATABASE_URL)],
        "general_settings:\n  master_key: secret-master-key\n",
    )
}

fn master_context(master_key: &SecretString) -> AuthContext {
    let input = StageInput::MasterKey(Some(master_key.clone()));
    let Ok(context) = dispatch(AuthBackend::MasterKey, &input, master_key) else {
        panic!("the master key stage authenticates");
    };
    context
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

fn application(config: &Config, rule: Arc<GrantRule>) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(MODEL_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(rule))
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

async fn run_request(config: &Config, rule: Arc<GrantRule>) -> (StatusCode, String, bool) {
    let reached = Arc::new(AtomicBool::new(false));
    let response = application(config, rule)
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

// the slot reads the same database switch the resource grant stage reads
fn assert_slot_settings() {
    let without = state_for(&config_with_master_key());
    assert!(!without.model_grant_intersection.database);
    let with = state_for(&config_with_database());
    assert!(with.model_grant_intersection.database);
}

fn assert_dispatch_reaches_the_stage() {
    let master_key = SecretString::from("secret-master-key".to_owned());
    let recorder = Arc::new(Recorder::default());
    let stage = GrantStageInput {
        context: master_context(&master_key),
        rule: rule(
            Some(Arc::new(ScriptedSources {
                team: TeamAnswer::Row,
            })),
            Arc::clone(&recorder),
        ),
        database: true,
        now: 0.0,
    };
    let input = StageInput::ModelGrantIntersection(Box::new(stage));
    let Ok(forwarded) = dispatch(AuthBackend::ModelGrantIntersection, &input, &master_key) else {
        panic!("the B732 constructor dispatches to the production grant stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");
    // the walk runs in the prefetch, so dispatch only hands on the identity it was given
    assert!(recorder.recorded().is_none());

    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(
            AuthBackend::ModelGrantIntersection,
            &wrong_stage,
            &master_key
        ),
        Err(StageFailure::Authentication(_))
    ));
}

// the registered stage walks the grants on a request with a database and reaches the handler
async fn assert_request_records_the_walk() {
    let recorder = Arc::new(Recorder::default());
    let (status, _body, reached) = run_request(
        &config_with_database(),
        rule(
            Some(Arc::new(ScriptedSources {
                team: TeamAnswer::Row,
            })),
            Arc::clone(&recorder),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(reached);
    let (allowlists, matched) = recorder.recorded().expect("the stage recorded its walk");
    assert_eq!(
        allowlists,
        vec![
            vec!["tier-a".to_owned()],
            vec!["tier-a".to_owned()],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ]
    );
    assert_eq!(matched, vec!["tier-a".to_owned()]);

    // without a database the walk still assembles the two lists LiteLLM returns and stamps nothing
    let skipped = Arc::new(Recorder::default());
    let (skip_status, _skip_body, skip_reached) = run_request(
        &config_with_master_key(),
        rule(
            Some(Arc::new(ScriptedSources {
                team: TeamAnswer::Missing,
            })),
            Arc::clone(&skipped),
        ),
    )
    .await;
    assert_eq!(skip_status, StatusCode::OK);
    assert!(skip_reached);
    let (skip_lists, skip_matched) = skipped.recorded().expect("the stage recorded its walk");
    assert_eq!(
        skip_lists,
        vec![vec!["tier-a".to_owned()], Vec::new()],
        "the no-database answer pairs the key list with the token's team models"
    );
    assert!(skip_matched.is_empty());

    // a database without the stage's sources skips the walk rather than failing the request
    let unwired = Arc::new(Recorder::default());
    let (unwired_status, _unwired_body, unwired_reached) =
        run_request(&config_with_database(), rule(None, Arc::clone(&unwired))).await;
    assert_eq!(unwired_status, StatusCode::OK);
    assert!(unwired_reached);
    assert!(unwired.recorded().is_none());
}

#[tokio::test]
async fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::ModelGrantIntersection));
    assert_slot_settings();
    assert_dispatch_reaches_the_stage();
    assert_request_records_the_walk().await;
}

#[tokio::test]
async fn failure_stops_downstream() {
    let recorder = Arc::new(Recorder::default());
    let (status, body, reached) = run_request(
        &config_with_database(),
        rule(
            Some(Arc::new(ScriptedSources {
                team: TeamAnswer::Missing,
            })),
            Arc::clone(&recorder),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("\"type\":\"auth_error\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"404\""), "{body}");
    assert!(body.contains("Team doesn't exist in db"), "{body}");
    assert!(!reached);
    assert!(recorder.recorded().is_none());
}

#[test]
fn stage_clock_reads_the_wall_clock() {
    let now = now_seconds();
    assert!(
        now > 1_600_000_000.0,
        "the stage clock reads epoch seconds: {now}"
    );
}

#[test]
fn rule_debug_names_the_walk_inputs() {
    let grant_rule = GrantRule {
        key: None,
        team: None,
        project: None,
        membership: None,
        membership_loaded: true,
        models: Some(vec!["gpt-4o".to_owned()]),
        sources: None,
        groups: None,
        sink: None,
    };
    let rendered = format!("{grant_rule:?}");
    assert!(rendered.starts_with("GrantRule"), "{rendered}");
    assert!(rendered.contains("gpt-4o"), "{rendered}");
}
