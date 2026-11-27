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
use secrecy::{ExposeSecret as _, SecretString};
use tower::ServiceExt as _;

use crate::auth::auth_identity::{AuthBackend, INSTALLED_STAGES, dispatch};
use crate::auth::parse_key_header;
use crate::routes;
use crate::routes::tests::state_for;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

use super::KeyLookupStageInput;

const CHAT_ROUTE: &str = "/v1/chat/completions";
const COMPLIANCE_ROUTE: &str = "/compliance/eu-ai-act";
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
    let state = state_for(&config());
    assert!(state.auth_key_lookup.authenticator.is_none());
}

#[tokio::test]
async fn failure_stops_downstream() {
    let missing = ScriptedAuthenticator::new(Answer::Missing);
    let missing_authenticator = Arc::clone(&missing);
    let (status, body, reached) = run_request(missing_authenticator).await;
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
    let outage_authenticator = Arc::clone(&outage);
    let (outage_status, outage_body, outage_reached) = run_request(outage_authenticator).await;
    assert_eq!(outage_status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        outage_body.contains("\"type\":\"no_db_connection\""),
        "{outage_body}"
    );
    assert!(outage_body.contains("\"code\":\"503\""), "{outage_body}");
    assert!(!outage_reached);
    assert_eq!(outage.calls(), vec![PRESENTED.to_owned()]);
}

// The compliance router merges after the state extension layer, so only the middleware's own copy can match it.
#[tokio::test]
async fn master_key_is_compared_outside_the_state_layer() {
    let application = application(&config(), ScriptedAuthenticator::new(Answer::Missing));
    let accepted = application
        .clone()
        .oneshot(compliance_request("secret-master-key"))
        .await
        .expect("the request reaches the router");
    assert_eq!(accepted.status(), StatusCode::OK);
    let refused = application
        .oneshot(compliance_request("not-the-master-key"))
        .await
        .expect("the request reaches the router");
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
}

fn compliance_request(key: &str) -> Request<Body> {
    Request::post(COMPLIANCE_ROUTE)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::from(
            r#"{"request_id":"r","user_id":"u","model":"m","timestamp":"t"}"#,
        ))
        .expect("the request builds")
}

#[test]
fn key_header_prefixes() {
    let litellm = parse_key_header("x-litellm-api-key", b"Bearer sk-litellm")
        .map(|key| key.expose_secret().to_owned());
    assert_eq!(litellm, Some("sk-litellm".to_owned()));
    let authorization =
        parse_key_header(header::AUTHORIZATION.as_str(), b"Bearer sk-authorization")
            .map(|key| key.expose_secret().to_owned());
    assert_eq!(authorization, Some("sk-authorization".to_owned()));
}

#[test]
fn stage_input_debug_redacts_keys() {
    let input = KeyLookupStageInput {
        presented: Some(SecretString::from("sk-presented-secret".to_owned())),
        authenticator: None,
        master_key: Some(SecretString::from("master-key-secret".to_owned())),
    };
    let text = format!("{input:?}");
    assert!(text.contains("presented: true"));
    assert!(!text.contains("sk-presented-secret"));
    assert!(!text.contains("master-key-secret"));
}

#[test]
fn authenticated_stage_forwarding() {
    let master = SecretString::from("master".to_owned());
    let Some(context) = super::master_key_context(Some(&master), &master) else {
        panic!("matching master key authenticates");
    };
    let input = super::StageInput::Authenticated(Box::new(context));
    let Ok(forwarded) = dispatch(AuthBackend::MasterKey, &input, &master) else {
        panic!("master stage forwards the authenticated key");
    };
    assert_eq!(forwarded.principal.subject, "master_key");
    dispatch(AuthBackend::AuthRequestPolicy, &input, &master).unwrap_err();
    let raw = super::StageInput::MasterKey(Some(master.clone()));
    dispatch(AuthBackend::AuthKeyLookup, &raw, &master).unwrap_err();
}

#[rstest::rstest]
#[case::empty("")]
#[case::wrong("other")]
fn master_stage_denials(#[case] value: &str) {
    let input = super::StageInput::MasterKey(Some(SecretString::from(value.to_owned())));
    let master = SecretString::from("master".to_owned());
    let Err(super::StageFailure::Authentication(failure)) =
        dispatch(AuthBackend::MasterKey, &input, &master)
    else {
        panic!("bad master credential is refused");
    };
    if value.is_empty() {
        assert!(matches!(*failure, super::AuthenticationFailure::Missing));
    } else {
        assert!(matches!(*failure, super::AuthenticationFailure::Invalid));
    }
}

#[test]
fn virtual_key_context_fields() {
    let fields = serde_json::from_str(r#"{"key_name":"key-name","user_id":"live-user","team_id":"live-team","team_alias":"Live","user_role":"proxy_admin"}"#).unwrap();
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap();
    let context = super::key_context(KeyLookupOutcome {
        auth,
        hashed_token: "hash".to_owned(),
    });
    assert_eq!(context.principal.subject, "live-user");
    assert_eq!(context.principal.teams[0].id, "live-team");
    assert_eq!(context.principal.teams[0].name.as_deref(), Some("Live"));
    assert_eq!(
        context.principal.credential_ref.token_id.as_deref(),
        Some("hash")
    );
    assert!(context.principal.source_key.0.is_some());
}

#[test]
fn master_input_before_lookup() {
    let request = Request::builder()
        .header(header::AUTHORIZATION, "Bearer master")
        .body(Body::empty())
        .unwrap();
    let super::StageInput::MasterKey(Some(key)) = super::master_stage_input(&request, None) else {
        panic!("the master stage extracts an unprocessed credential");
    };
    assert_eq!(key.expose_secret(), "master");
}
