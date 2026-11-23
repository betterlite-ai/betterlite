#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_org_lookup::error::{OrgFailure, OrgLookupError};
use betterlite_auth::auth_org_lookup::organization::OrgLookup;
use betterlite_auth::auth_org_lookup::ports::{OrgCache, OrgStore, StoreAnswer};
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::auth_read::table::AuthReadError;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_db::database_availability::message::TRANSIENT_MESSAGE;
use betterlite_db::database_availability::policy::{AllowRequestsOnOutage, PolicyError};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::resource_rows::organization::OrganizationTable;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::{OrgLookupForRequest, OrgRule, OrgStageInput};
use crate::auth::auth_identity::{
    AuthBackend, AuthContext, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::routes;
use crate::routes::tests::state_for;

// a recognized route, so the auth chain runs before the marking handler
const MODEL_ROUTE: &str = "/v1/files";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Found,
    Outage,
    Unsupported,
}

#[derive(Debug)]
struct ScriptedLookup {
    answer: Answer,
    calls: Mutex<Vec<String>>,
}

impl ScriptedLookup {
    fn new(answer: Answer) -> Arc<Self> {
        Arc::new(Self {
            answer,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("the lookup calls").clone()
    }
}

fn org() -> OrganizationTable {
    serde_json::from_str(
        r#"{"organization_id": "o1", "organization_alias": "acme", "budget_id": "b1", "created_by": "a", "updated_by": "a"}"#,
    )
    .expect("the row fixture parses")
}

#[async_trait]
impl OrgLookupForRequest for ScriptedLookup {
    async fn org_object(
        &self,
        org_id: &str,
        _policy: &AllowRequestsOnOutage,
    ) -> Result<Option<OrganizationTable>, OrgLookupError> {
        self.calls
            .lock()
            .expect("the lookup calls")
            .push(org_id.to_owned());
        match self.answer {
            Answer::Found => Ok(Some(org())),
            // the pool fault a denied outage raises, and the wrong-typed setting a policy read rejects
            Answer::Outage => Err(OrgLookupError::Failure(OrgFailure::Store(Box::new(
                AuthReadError::Repository(DbError::NoDbConnected),
            )))),
            Answer::Unsupported => Err(OrgLookupError::Failure(OrgFailure::Policy(
                PolicyError::UnsupportedValue,
            ))),
        }
    }
}

fn rule(lookup: Arc<ScriptedLookup>, org_id: Option<&str>) -> Arc<OrgRule> {
    Arc::new(OrgRule {
        org_id: org_id.map(str::to_owned),
        lookup: Some(lookup),
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

fn application(config: &Config, rule: Arc<OrgRule>) -> Router {
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

async fn run_request(config: &Config, rule: Arc<OrgRule>) -> (StatusCode, String, bool) {
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

#[tokio::test]
async fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthOrgLookup));
    // the slot carries DEFAULT_IN_MEMORY_TTL and the outage setting the stage reads
    let without = state_for(&config_with_master_key());
    assert!(!without.auth_org_lookup.database);
    assert_eq!(
        without.auth_org_lookup.in_memory_ttl.to_bits(),
        5.0_f64.to_bits()
    );
    assert_eq!(
        without.auth_org_lookup.allow_requests_on_outage,
        AllowRequestsOnOutage::Boolean(false)
    );
    let configured = config(
        &[
            ("DATABASE_URL", DATABASE_URL),
            ("DEFAULT_IN_MEMORY_TTL", "45"),
        ],
        "general_settings:\n  master_key: secret-master-key\n  allow_requests_on_db_unavailable: \"true\"\n",
    );
    let with = state_for(&configured);
    assert!(with.auth_org_lookup.database);
    assert_eq!(
        with.auth_org_lookup.in_memory_ttl.to_bits(),
        45.0_f64.to_bits()
    );
    assert_eq!(
        with.auth_org_lookup.allow_requests_on_outage,
        AllowRequestsOnOutage::Text("true".to_owned())
    );

    let master_key = SecretString::from("secret-master-key".to_owned());
    let stage = OrgStageInput {
        context: master_context(&master_key),
        rule: rule(ScriptedLookup::new(Answer::Found), Some("o1")),
        policy: AllowRequestsOnOutage::Boolean(false),
        database: true,
    };
    let input = StageInput::AuthOrgLookup(Box::new(stage));
    let Ok(forwarded) = dispatch(AuthBackend::AuthOrgLookup, &input, &master_key) else {
        panic!("the B732 constructor dispatches to the production organization stage");
    };
    assert_eq!(forwarded.principal.subject, "master_key");

    let wrong_stage = StageInput::MasterKey(Some(master_key.clone()));
    assert!(matches!(
        dispatch(AuthBackend::AuthOrgLookup, &wrong_stage, &master_key),
        Err(StageFailure::Authentication(_))
    ));

    // the registered stage runs the lookup on a request with a database and reaches the handler
    let lookup = ScriptedLookup::new(Answer::Found);
    let (status, _body, reached) = run_request(
        &config_with_database(),
        rule(Arc::clone(&lookup), Some("o1")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(reached);
    assert_eq!(lookup.calls(), vec!["o1".to_owned()]);

    // without a database the same rule reads nothing and the request still reaches the handler
    let skipped = ScriptedLookup::new(Answer::Outage);
    let (skip_status, _skip_body, skip_reached) = run_request(
        &config_with_master_key(),
        rule(Arc::clone(&skipped), Some("o1")),
    )
    .await;
    assert_eq!(skip_status, StatusCode::OK);
    assert!(skip_reached);
    assert!(skipped.calls().is_empty());
}

#[tokio::test]
async fn failure_stops_downstream() {
    let outage = ScriptedLookup::new(Answer::Outage);
    let (status, body, reached) = run_request(
        &config_with_database(),
        rule(Arc::clone(&outage), Some("o1")),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains("\"type\":\"no_db_connection\""), "{body}");
    assert!(body.contains("\"param\":\"None\""), "{body}");
    assert!(body.contains("\"code\":\"503\""), "{body}");
    assert!(body.contains(TRANSIENT_MESSAGE), "{body}");
    assert!(!reached);
    assert_eq!(outage.calls(), vec!["o1".to_owned()]);

    let unsupported = ScriptedLookup::new(Answer::Unsupported);
    let (failure_status, failure_body, failure_reached) = run_request(
        &config_with_database(),
        rule(Arc::clone(&unsupported), Some("o1")),
    )
    .await;
    assert_eq!(failure_status, StatusCode::UNAUTHORIZED);
    assert!(
        failure_body.contains("\"type\":\"auth_error\""),
        "{failure_body}"
    );
    assert!(failure_body.contains("\"code\":\"401\""), "{failure_body}");
    assert!(
        failure_body.contains(
            "Authentication Error, general_settings.allow_requests_on_db_unavailable must be a boolean or a string"
        ),
        "{failure_body}"
    );
    assert!(!failure_reached);
}

// the cache trait's fallible answer, so each NoCache signature stays one line
type CacheAnswer<T> = Result<T, AuthCacheError>;

// the cache the real lookup is built with when a test only needs the domain answer
struct NoCache;

#[async_trait]
impl OrgCache for NoCache {
    async fn read_org(
        &self,
        _key: &str,
        _local_only: bool,
    ) -> CacheAnswer<Option<OrganizationTable>> {
        Ok(None)
    }

    async fn write_org(&self, _key: &str, _org: &OrganizationTable, _ttl: f64) -> CacheAnswer<()> {
        Ok(())
    }

    async fn read_project(&self, _key: &str) -> CacheAnswer<Option<CachedProject>> {
        Ok(None)
    }

    async fn write_project(
        &self,
        _key: &str,
        _project: &CachedProject,
        _ttl: f64,
    ) -> CacheAnswer<()> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

// the row a database would answer, so the domain lookup comes back with a filled organization
struct OneStore;

#[async_trait]
impl OrgStore for OneStore {
    async fn find_org(&self, _org_id: &str, _include_budget: bool) -> StoreAnswer<Option<String>> {
        Ok(Some(
            serde_json::to_string(&org()).expect("the row serializes"),
        ))
    }

    async fn find_by_alias(&self, _org_alias: &str) -> StoreAnswer<Vec<String>> {
        Ok(Vec::new())
    }

    async fn find_project(&self, _project_id: &str) -> StoreAnswer<Option<String>> {
        Ok(None)
    }
}

#[tokio::test]
async fn the_trait_runs_the_production_lookup() {
    let store: Arc<dyn OrgStore> = Arc::new(OneStore);
    let lookup = OrgLookup {
        store: Some(store),
        cache: Arc::new(NoCache),
        in_memory_ttl: 5.0_f64,
    };
    // the stored row comes back through the trait, so a stub answer cannot stand in for the delegation
    let found = lookup
        .org_object("o1", &AllowRequestsOnOutage::Boolean(false))
        .await
        .unwrap()
        .expect("the stored row is found");
    assert_eq!(found.organization_id.as_deref(), Some("o1"));
}

#[test]
fn rule_debug_shows_the_org_id() {
    let rule = OrgRule {
        org_id: Some("o1".to_owned()),
        lookup: None,
    };
    let rendered = format!("{rule:?}");
    assert!(rendered.starts_with("OrgRule"), "{rendered}");
    assert!(rendered.contains("\"o1\""), "{rendered}");
}
