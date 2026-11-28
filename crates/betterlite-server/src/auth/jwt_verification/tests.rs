#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Extension;
use axum::http::{Request, StatusCode, header};
use axum::routing::get;
use betterlite_auth::auth_cache::AuthCacheError;
use betterlite_auth::auth_identity::roles::AuthMethod;
use betterlite_auth::jwt_keys::cache::KeyCache;
use betterlite_auth::jwt_keys::clock::Clock;
use betterlite_auth::jwt_keys::errors::KeyError;
use betterlite_auth::jwt_keys::http::FetchError;
use betterlite_auth::jwt_verification::settings::JwtVerificationSettings;
use betterlite_auth::jwt_verification::source::PublicKeySource;
use betterlite_auth::jwt_verification::userinfo::{UserinfoHttp, UserinfoReply};
use betterlite_auth::jwt_verification::verifier::JwtVerifier;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::jwt_settings::Settings as JwtAuthSettings;
use betterlite_config::types::Config;
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::stage_input;
use crate::auth::auth_identity::{
    AuthBackend, INSTALLED_STAGES, StageFailure, StageInput, dispatch,
};
use crate::auth::prefetch_stage;
use crate::routes;
use crate::routes::tests::state_for;

const AUDIENCE: &str = "litellm-proxy";
const TOKEN: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6InJzYS1rZXkiLCJ0eXAiOiJKV1QifQ.eyJzdWIiOiJ1c2VyLTEiLCJleHAiOjQxMDI0NDQ4MDAsImlhdCI6MTYwMDAwMDAwMCwibmJmIjoxNjAwMDAwMDAwLCJhdWQiOiJsaXRlbGxtLXByb3h5In0.lmtuoI42BHAJO7eRUfyQuRDrqtqFJFvh7El5zvxfuAmysIPQOgKKwU029eH27EJxdk_-EZbcT8u-HzBaJODYSA_4YtzFcmK32xZSoGNDfXykWXAxEg_5EdGZ6f22mUnR3ANFa2-mx4RbOa0pdYXv6QVhqUL1nlDesO5NABl18M2b3xrhDF93Qi0-ebKzNg7FBbTF6wTox6Q8xUuw4jbZjxFEYe6523OPMDfBd4qRn0tEmDjzmFncEz08CkJY7gtGI30mBBt_Y_XtROy0ix9ayJOvRnH_9CTR_sgHb0EkIQGtIONz9egztReqqSxt2TrI_G8ymZpKpENjf3pC_h0Mfw";
const RSA_JWK: &str = r#"{"kty":"RSA","kid":"rsa-key","n":"qBV4uuD4ho4EljGE_P0Mkeu4ez_KwlFScpDLtRQDxOg8Xmq-QcuML77h593QZ-K_rbWxHqD_VVHUQLKTiLuIJNeUS6QXqpbbkvwhJpNNyUFBr0d46RnnU6kQF5wH4KIfWYs2ns97lceqOLc40a8eppoap6k1aXr2l_b2NuJvKPB7qmeAbGu9qvLKzOIhqUGCjcVZFr75IXrVDrX8VXkN3LIBgtdFIbz89AGfh8hV4iNbGmCGWJxadJENmbNCmqNt0GS1zJsl9fK7ivXVnZbT8psHOxnmipU_DjXvxhIoBSZH9b1F0mrBdejxM3rP8BZPPpcZPC_sKzMyNwxTeRJQhw","e":"AQAB"}"#;

#[derive(Default)]
struct MemoryCache {
    entries: Mutex<BTreeMap<String, Json>>,
}

#[async_trait]
impl KeyCache for MemoryCache {
    async fn read_value(&self, key: &str) -> Result<Option<Json>, AuthCacheError> {
        Ok(self.entries.lock().unwrap().get(key).cloned())
    }

    async fn write_value(
        &self,
        key: &str,
        value: &Json,
        _ttl_seconds: f64,
    ) -> Result<(), AuthCacheError> {
        self.entries
            .lock()
            .unwrap()
            .insert(key.to_owned(), value.clone());
        Ok(())
    }
}

struct NoUserinfoHttp;

#[async_trait]
impl UserinfoHttp for NoUserinfoHttp {
    async fn get(
        &self,
        _url: &str,
        _authorization: &str,
        _accept: &str,
    ) -> Result<UserinfoReply, FetchError> {
        Err(FetchError::Transport(HttpError::Timeout))
    }

    fn close(&self) {}
}

struct FrozenClock;

#[async_trait]
impl Clock for FrozenClock {
    fn now(&self) -> f64 {
        1_700_000_000.0
    }

    async fn sleep(&self, _delay: Duration) {}
}

fn verifier() -> Arc<JwtVerifier> {
    let settings = JwtVerificationSettings {
        audience: Some(AUDIENCE.to_owned()),
        issuer: None,
        jwt: JwtAuthSettings::default(),
    };
    Arc::new(JwtVerifier::new(
        Arc::new(settings),
        Arc::new(MemoryCache::default()),
        Arc::new(NoUserinfoHttp),
        Arc::new(FrozenClock),
    ))
}

enum SourceReply {
    Key(Json),
    Unreachable,
}

struct FixedSource {
    reply: SourceReply,
}

#[async_trait]
impl PublicKeySource for FixedSource {
    async fn resolve_public_key(&self, _kid: Option<&str>) -> Result<Json, KeyError> {
        match &self.reply {
            SourceReply::Key(key) => Ok(key.clone()),
            SourceReply::Unreachable => Err(KeyError::Unreachable {
                message: "the identity provider is down".to_owned(),
            }),
        }
    }

    async fn resolve_keys_for_url(
        &self,
        _jwks_url: &str,
        _kid: Option<&str>,
    ) -> Result<Json, KeyError> {
        self.resolve_public_key(None).await
    }
}

fn source(reply: SourceReply) -> Arc<dyn PublicKeySource> {
    Arc::new(FixedSource { reply })
}

fn jwk() -> Json {
    serde_json::from_str(RSA_JWK).unwrap()
}

fn bad_signature_token() -> String {
    let (unsigned, _) = TOKEN.rsplit_once('.').unwrap();
    format!("{unsigned}.AAAA")
}

fn request_with(token: &str, reply: SourceReply) -> Request<Body> {
    Request::get("/v1/chat/completions")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .extension(verifier())
        .extension(source(reply))
        .body(Body::empty())
        .unwrap()
}

async fn reached_handler(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

fn chat_application(config: &Config) -> Router {
    let chat = Router::new().route("/v1/chat/completions", get(reached_handler));
    routes::router(
        state_for(config),
        Arc::new(AtomicBool::new(false)),
        chat,
        Router::new(),
    )
}

#[test]
fn stage_input_debug_marks_presence_without_the_token() {
    let request = request_with(TOKEN, SourceReply::Key(jwk()));
    let StageInput::JwtVerification(stage) = stage_input(&request) else {
        panic!("the JWT verification layer keeps its own stage input");
    };
    let debug = format!("{stage:?}");
    assert!(debug.contains("verifier_present: true"), "{debug}");
    assert!(debug.contains("source_present: true"), "{debug}");
    assert!(debug.contains("token_present: true"), "{debug}");
    assert!(debug.contains("verified: false"), "{debug}");
    assert!(!debug.contains(TOKEN), "{debug}");
}

#[tokio::test]
async fn stage_is_registered() {
    assert_eq!(
        INSTALLED_STAGES.first(),
        Some(&AuthBackend::JwtKeys),
        "the key fetch still runs first"
    );
    assert_eq!(
        INSTALLED_STAGES.get(1),
        Some(&AuthBackend::JwtVerification),
        "the signature check runs right after the key fetch"
    );
    let lookup = INSTALLED_STAGES
        .iter()
        .position(|stage| *stage == AuthBackend::AuthKeyLookup)
        .unwrap();
    assert!(
        lookup > 1,
        "the verification stage stops a token before the identity lookup"
    );

    let master_key = SecretString::from("secret-master-key".to_owned());
    let request = request_with(TOKEN, SourceReply::Key(jwk()));
    let Ok(prefetched) = prefetch_stage(stage_input(&request)).await else {
        panic!("a valid signed token passes the verification stage");
    };
    let Ok(context) = dispatch(AuthBackend::JwtVerification, &prefetched, &master_key) else {
        panic!("the verification stage hands on the verified identity");
    };
    assert_eq!(context.principal.auth_method, AuthMethod::BearerJwt);
    assert_eq!(context.principal.audience, vec![AUDIENCE.to_owned()]);
    assert!(context.principal.issuer.is_none());

    let refused = request_with(&bad_signature_token(), SourceReply::Key(jwk()));
    let Err(failure) = prefetch_stage(stage_input(&refused)).await else {
        panic!("a tampered signature never passes the verification stage");
    };
    let StageFailure::JwtVerification(exception) = failure else {
        panic!("the verification stage raises its own failure");
    };
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.code, "401");
}

async fn failure_response(
    app: Router,
    reached: &Arc<AtomicBool>,
    token: &str,
    reply: SourceReply,
) -> (StatusCode, String) {
    let request = Request::get("/v1/chat/completions")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .extension(Arc::clone(reached))
        .extension(verifier())
        .extension(source(reply))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

fn downstream_application() -> (Router, Arc<AtomicBool>) {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings:\n  master_key: secret-master-key\n")
    .unwrap();
    let reached = Arc::new(AtomicBool::new(false));
    (chat_application(&config), reached)
}

#[tokio::test]
async fn failure_stops_downstream() {
    let (app, reached) = downstream_application();

    let (refused_status, refused_text) = failure_response(
        app.clone(),
        &reached,
        &bad_signature_token(),
        SourceReply::Key(jwk()),
    )
    .await;
    assert_eq!(refused_status, StatusCode::UNAUTHORIZED);
    assert!(refused_text.contains("auth_error"), "{refused_text}");
    assert!(refused_text.contains("\"code\":\"401\""), "{refused_text}");
    assert!(
        refused_text.contains("Validation fails: Signature verification failed"),
        "{refused_text}"
    );
    assert!(
        !reached.load(Ordering::Acquire),
        "the refused signature reached the handler"
    );

    let (outage_status, outage_text) =
        failure_response(app, &reached, TOKEN, SourceReply::Unreachable).await;
    assert_eq!(outage_status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        outage_text.contains("auth_provider_unavailable"),
        "{outage_text}"
    );
    assert!(outage_text.contains("\"code\":\"503\""), "{outage_text}");
    assert!(
        !reached.load(Ordering::Acquire),
        "the unavailable key reached the handler"
    );
}
