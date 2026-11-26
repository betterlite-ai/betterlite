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
use betterlite_auth::auth_key_lookup::error::KeyLookupError;
use betterlite_auth::auth_key_lookup::{KeyAuthenticator, KeyLookupOutcome};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::{DbError, ErrorType};
use tower::ServiceExt as _;

use crate::auth::auth_identity::{AuthBackend, INSTALLED_STAGES};
use crate::routes;
use crate::routes::tests::state_for;

const CHAT_ROUTE: &str = "/v1/chat/completions";
const PRESENTED: &str = "sk-virtual-key";

#[derive(Clone, Copy)]
enum Answer {
    Missing,
    Outage,
}

struct ScriptedAuthenticator {
    answer: Answer,
    calls: Mutex<Vec<String>>,
}

impl ScriptedAuthenticator {
    fn new(answer: Answer) -> Arc<Self> {
        Arc::new(Self {
            answer,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("the calls lock").clone()
    }
}

#[async_trait]
impl KeyAuthenticator for ScriptedAuthenticator {
    async fn authenticate(&self, presented: &str) -> Result<KeyLookupOutcome, KeyLookupError> {
        self.calls
            .lock()
            .expect("the calls lock")
            .push(presented.to_owned());
        match self.answer {
            Answer::Missing => Err(KeyLookupError::MissingKey {
                key: "hashed-virtual-key".to_owned(),
            }),
            Answer::Outage => Err(KeyLookupError::Lookup(DbLookupError::Database(DbError {
                error_type: ErrorType::Connect,
                message: "Can't reach database server at localhost:5432".to_owned(),
                code: None,
                meta_code: None,
                traceback_modules: Vec::new(),
                cause: None,
            }))),
        }
    }
}

fn config() -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("general_settings:\n  master_key: secret-master-key\n")
    .expect("the test config loads")
}

async fn mark_called(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

fn application(config: &Config, authenticator: Arc<dyn KeyAuthenticator>) -> Router {
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        Router::new().route(CHAT_ROUTE, post(mark_called)),
        Router::new(),
    )
    .layer(Extension(authenticator))
}

async fn run_request(authenticator: Arc<dyn KeyAuthenticator>) -> (StatusCode, String, bool) {
    let reached = Arc::new(AtomicBool::new(false));
    let mut request = Request::builder()
        .method("POST")
        .uri(CHAT_ROUTE)
        .header(header::AUTHORIZATION, format!("Bearer {PRESENTED}"))
        .body(Body::empty())
        .expect("the request builds");
    request.extensions_mut().insert(Arc::clone(&reached));
    let response = application(&config(), authenticator)
        .oneshot(request)
        .await
        .expect("the request reaches the router");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the response body is readable");
    let text = String::from_utf8(body.to_vec()).expect("the response body is UTF-8");
    (status, text, reached.load(Ordering::Acquire))
}

#[test]
fn stage_is_registered() {
    assert!(INSTALLED_STAGES.contains(&AuthBackend::AuthKeyLookup));
    assert_eq!(INSTALLED_STAGES.first(), Some(&AuthBackend::AuthKeyLookup));
    let state = state_for(&config());
    assert!(state.auth_key_lookup.authenticator.is_none());
}

#[tokio::test]
async fn failure_stops_downstream() {
    let missing = ScriptedAuthenticator::new(Answer::Missing);
    let (status, body, reached) = run_request(Arc::clone(&missing)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body.contains("\"type\":\"token_not_found_in_db\""),
        "{body}"
    );
    assert!(body.contains("\"param\":\"key\""), "{body}");
    assert!(body.contains("\"code\":\"401\""), "{body}");
    assert!(!reached);
    assert_eq!(missing.calls(), vec![PRESENTED.to_owned()]);

    let outage = ScriptedAuthenticator::new(Answer::Outage);
    let (status, body, reached) = run_request(Arc::clone(&outage)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains("\"type\":\"no_db_connection\""), "{body}");
    assert!(body.contains("\"code\":\"503\""), "{body}");
    assert!(!reached);
    assert_eq!(outage.calls(), vec![PRESENTED.to_owned()]);
}
