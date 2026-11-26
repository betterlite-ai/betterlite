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
use betterlite_auth::jwt_keys::http::{FetchError, HttpReply, JwksHttp};
use betterlite_auth::jwt_keys::resolver::JwksResolver;
use betterlite_auth::jwt_keys::settings::JwtKeySettings;
use betterlite_config::loader::ConfigLoader;
use betterlite_http::error::HttpError;
use betterlite_models::decisions::Json;
use rstest::rstest;
use secrecy::SecretString;
use tower::ServiceExt as _;

use super::JwtKeyStageInput;
use crate::auth::auth_identity::{AuthBackend, INSTALLED_STAGES, StageInput, dispatch};
use crate::auth::prefetch_stage;
use crate::routes;
use crate::routes::tests::state_for;

const TOKEN: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6ImsxIn0.eyJzdWIiOiJ1c2VyLTEifQ.c2ln";

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

struct MatchingJwks;

#[async_trait]
impl JwksHttp for MatchingJwks {
    async fn get(&self, _url: &str) -> Result<HttpReply, FetchError> {
        Ok(HttpReply {
            status: 200,
            body: br#"{"keys":[{"kid":"k1","kty":"RSA","n":"AQAB"}]}"#.to_vec(),
        })
    }
}

struct UnreachableJwks;

#[async_trait]
impl JwksHttp for UnreachableJwks {
    async fn get(&self, _url: &str) -> Result<HttpReply, FetchError> {
        Err(FetchError::Transport(HttpError::Timeout))
    }
}

struct FrozenClock;

#[async_trait]
impl Clock for FrozenClock {
    fn now(&self) -> f64 {
        1_700_000_000.0
    }

    async fn sleep(&self, _delay: Duration) {}
}

fn resolver_with(http: Arc<dyn JwksHttp>) -> Arc<JwksResolver> {
    let settings = JwtKeySettings::new(
        Some("https://idp.example.com/jwks".to_owned()),
        600.0,
        3_600.0,
        None,
        None,
    )
    .unwrap();
    let cache: Arc<dyn KeyCache> = Arc::new(MemoryCache::default());
    let clock: Arc<dyn Clock> = Arc::new(FrozenClock);
    Arc::new(JwksResolver::new(Arc::new(settings), cache, http, clock))
}

async fn reached_handler(Extension(reached): Extension<Arc<AtomicBool>>) -> &'static str {
    reached.store(true, Ordering::Release);
    "reached"
}

#[tokio::test]
async fn stage_is_registered() {
    assert_eq!(INSTALLED_STAGES.first(), Some(&AuthBackend::JwtKeys));
    let master_key = SecretString::from("secret-master-key".to_owned());
    let http: Arc<dyn JwksHttp> = Arc::new(MatchingJwks);
    let input = StageInput::JwtKeys(Box::new(JwtKeyStageInput {
        resolver: Some(resolver_with(http)),
        token: Some(SecretString::from(TOKEN.to_owned())),
    }));
    let Ok(prefetched) = prefetch_stage(input).await else {
        panic!("the JWT key stage fetches the matching key");
    };
    let Ok(context) = dispatch(AuthBackend::JwtKeys, &prefetched, &master_key) else {
        panic!("the JWT key stage yields the identity the master-key stage replaces");
    };
    assert_eq!(context.principal.auth_method, AuthMethod::BearerJwt);
}

#[test]
fn debug_keeps_the_token_out() {
    let input = JwtKeyStageInput {
        resolver: None,
        token: Some(SecretString::from(TOKEN.to_owned())),
    };
    let debug = format!("{input:?}");
    assert!(debug.contains("token_present: true"), "{debug}");
    assert!(!debug.contains(TOKEN), "{debug}");
}

#[rstest]
#[case::empty("")]
#[case::plain_key("secret-master-key")]
#[case::two_segments("one.two")]
#[case::four_segments("one.two.three.four")]
#[tokio::test]
async fn non_jwt_credentials_skip_key_fetch(#[case] token: &str) {
    let http: Arc<dyn JwksHttp> = Arc::new(UnreachableJwks);
    let input = StageInput::JwtKeys(Box::new(JwtKeyStageInput {
        resolver: Some(resolver_with(http)),
        token: Some(SecretString::from(token.to_owned())),
    }));
    assert!(matches!(
        prefetch_stage(input).await,
        Ok(StageInput::JwtKeys(_))
    ));
}

#[tokio::test]
async fn failure_stops_downstream() {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("general_settings:\n  master_key: secret-master-key\n")
    .unwrap();
    let reached = Arc::new(AtomicBool::new(false));
    let chat = Router::new().route("/v1/chat/completions", get(reached_handler));
    let application = routes::router(
        state_for(&config),
        Arc::new(AtomicBool::new(false)),
        chat,
        Router::new(),
    );
    let http: Arc<dyn JwksHttp> = Arc::new(UnreachableJwks);
    let request = Request::get("/v1/chat/completions")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .extension(resolver_with(http))
        .extension(Arc::clone(&reached))
        .body(Body::empty())
        .unwrap();
    let response = application.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("auth_provider_unavailable"), "{text}");
    assert!(text.contains("\"param\":\"None\""), "{text}");
    assert!(text.contains("\"code\":\"503\""), "{text}");
    assert!(
        !reached.load(Ordering::Acquire),
        "an unavailable key must stop before the downstream handler"
    );
}
