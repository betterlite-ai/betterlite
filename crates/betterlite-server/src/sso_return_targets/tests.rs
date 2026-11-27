#![cfg(test)]

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, Response, StatusCode, header};
use betterlite_auth::auth_cache::{AuthCache, gateway_cache};
use betterlite_auth::trusted_network::settings::TrustedNetworkSettings;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use rstest::rstest;

use super::SsoReturnTargetError;
use super::callback::get_redirect_url_for_sso;
use super::cookie::{
    RETURN_TO_COOKIE, delete_return_to_cookie, is_secure_request, persist_return_to_cookie,
    return_to_cookie, session_token_cookie, set_session_token_cookie,
};
use super::decode::decode_sso_return_target;
use super::login_code::{
    CacheWriteError, LOGIN_CODE_PREFIX, LoginCode, LoginCodeCache, LoginCodeCaches,
    generate_login_code,
};
use super::origin::{Origin, origin, validate_return_to};
use super::redirect::{ReturnToRedirect, sso_return_to_redirect};
use super::return_path::is_same_origin_return_path;
use crate::sso_pkce::VerifierEntropy;

const NOW: i64 = 1_700_000_000;
const JWT: &str = "jwt.token-value";
const CONTROL_PLANE: &str = "https://cp.example.com";
const DELETED: &str = "litellm_cp_return_to=\"\"; expires=Tue, 14 Nov 2023 22:13:20 GMT; Max-Age=0; Path=/; SameSite=lax";

static FIXED_ENTROPY: FixedEntropy = FixedEntropy;

struct FixedEntropy;

impl VerifierEntropy for FixedEntropy {
    fn fill(&self, bytes: &mut [u8]) -> Result<(), getrandom::Error> {
        for (position, slot) in bytes.iter_mut().enumerate() {
            *slot = u8::try_from(position).unwrap();
        }
        Ok(())
    }
}

struct FailingEntropy;

impl VerifierEntropy for FailingEntropy {
    fn fill(&self, _bytes: &mut [u8]) -> Result<(), getrandom::Error> {
        Err(getrandom::Error::UNSUPPORTED)
    }
}

#[derive(Default)]
struct RecordingCache {
    writes: Mutex<Vec<Write>>,
    fails: bool,
}

impl RecordingCache {
    fn failing() -> Self {
        Self {
            writes: Mutex::new(Vec::new()),
            fails: true,
        }
    }
}

struct Write {
    key: String,
    token: String,
    redirect_url: String,
    ttl_seconds: f64,
}

#[async_trait]
impl LoginCodeCache for RecordingCache {
    async fn store_login_code(
        &self,
        cache_key: &str,
        code: &LoginCode<'_>,
        ttl_seconds: f64,
    ) -> Result<(), CacheWriteError> {
        if self.fails {
            return Err(CacheWriteError::RedisUsage(
                RedisCacheValuesError::TtlOutOfRange { seconds: 0.0_f64 },
            ));
        }
        self.writes.lock().unwrap().push(Write {
            key: cache_key.to_owned(),
            token: code.token.to_owned(),
            redirect_url: code.redirect_url.to_owned(),
            ttl_seconds,
        });
        Ok(())
    }
}

fn request(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn https_request() -> Request<Body> {
    request("https://proxy.example.com/sso/key/generic")
}

fn http_request() -> Request<Body> {
    request("http://proxy.example.com/sso/key/generic")
}

fn https_settings() -> TrustedNetworkSettings {
    TrustedNetworkSettings {
        proxy_base_url: "https://proxy.example.com".to_owned(),
        use_x_forwarded_for: false,
        trusted_proxy_ranges: None,
        mcp_trusted_proxy_ranges: None,
        mcp_xff_num_trusted_hops: None,
    }
}

fn header_values(response: &Response<Body>, name: header::HeaderName) -> Vec<String> {
    response
        .headers()
        .get_all(name)
        .iter()
        .map(|value| value.to_str().unwrap().to_owned())
        .collect()
}

fn redirect_to<'request, Redis, User>(
    return_to: Option<&'request str>,
    control_plane_url: Option<&'request str>,
    request: &'request Request<Body>,
    trusted_network: &'request TrustedNetworkSettings,
    caches: LoginCodeCaches<'request, Redis, User>,
) -> ReturnToRedirect<'request, Redis, User, FixedEntropy>
where
    Redis: LoginCodeCache,
    User: LoginCodeCache,
{
    ReturnToRedirect {
        request,
        trusted_network,
        now_unix: NOW,
        return_to,
        jwt_token: JWT,
        control_plane_url,
        caches,
        entropy: &FIXED_ENTROPY,
    }
}

// both cache slots share this test's recorder type unless the case supplies a Redis cache
fn login_caches<'caches, Cache: LoginCodeCache>(
    user: &'caches Cache,
    redis: Option<&'caches Cache>,
) -> LoginCodeCaches<'caches, Cache, Cache> {
    LoginCodeCaches { redis, user }
}

fn clock() -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(|| 1_000.0_f64)
}

fn local_auth_cache() -> AuthCache<DualCache<LocalMemoryOps>> {
    gateway_cache(
        Arc::new(LocalMemoryOps::new(1024, clock()).unwrap()),
        Arc::new(LocalMemoryOps::new(1024, clock()).unwrap()),
        None,
        DualCacheSettings {
            default_in_memory_ttl: None,
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: 1000,
            redis_delete_chunk_size: 1000,
        },
        45.0_f64,
    )
}

struct SilentHooks;

impl ServiceHooks for SilentHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn StdError + Send + Sync)) {}
}

struct OpenGate;

impl BreakerGate for OpenGate {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Err(BreakerOpen { name })
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn StdError + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn StdError + Send + Sync)) {}
}

// The breaker refuses every operation before a connection opens, so the write returns the Redis error itself
async fn open_breaker_redis_cache() -> RedisCache {
    let options = RedisCacheOptions {
        overrides: RedisOverrides::default(),
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: 60,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(SilentHooks),
        gate: Arc::new(OpenGate),
    };
    let env = BTreeMap::from([("REDIS_URL".to_owned(), "redis://127.0.0.1:1/0".to_owned())]);
    connect(options, &env).await.unwrap()
}

#[rstest]
#[case::root("/", true)]
#[case::relative("/dashboard?tab=1", true)]
#[case::unicode("/résumé", true)]
#[case::bare("dashboard", false)]
#[case::empty("", false)]
#[case::protocol_relative("//evil.example", false)]
#[case::triple_slash("///evil.example", false)]
#[case::backslash("/\\evil.example", false)]
#[case::space("/a b", false)]
#[case::tab("/a\tb", false)]
#[case::newline("/a\nb", false)]
#[case::del("/a\u{7f}b", false)]
fn accepts_only_strict_relative_paths(#[case] return_to: &str, #[case] expected: bool) {
    assert_eq!(is_same_origin_return_path(return_to), expected);
}

#[rstest]
#[case::https_default_port("https://cp.example.com/x", "https", "cp.example.com", 443)]
#[case::http_default_port("http://cp.example.com", "http", "cp.example.com", 80)]
#[case::explicit_port("https://CP.Example.COM:8443/x", "https", "cp.example.com", 8443)]
#[case::no_scheme("/dashboard", "", "", 80)]
#[case::ipv6("http://[::1]/x", "http", "::1", 80)]
fn builds_the_control_plane_origin(
    #[case] url: &str,
    #[case] scheme: &str,
    #[case] host: &str,
    #[case] port: u16,
) {
    assert_eq!(
        origin(url).unwrap(),
        Origin {
            scheme: scheme.to_owned(),
            host: host.to_owned(),
            port,
        }
    );
}

#[rstest]
#[case::unconfigured("https://cp.example.com/x", None, Ok(false))]
#[case::matching("https://cp.example.com/x", Some(CONTROL_PLANE), Ok(true))]
#[case::host_case("HTTPS://CP.EXAMPLE.COM:443/x", Some(CONTROL_PLANE), Ok(true))]
#[case::http_port("http://cp.example.com:80/x", Some("http://cp.example.com"), Ok(true))]
#[case::wrong_host("https://other.example.com/x", Some(CONTROL_PLANE), Err(()))]
#[case::wrong_scheme("http://cp.example.com/x", Some(CONTROL_PLANE), Err(()))]
#[case::wrong_port("https://cp.example.com:8443/x", Some(CONTROL_PLANE), Err(()))]
fn validates_absolute_return_targets(
    #[case] return_to: &str,
    #[case] control_plane_url: Option<&str>,
    #[case] expected: Result<bool, ()>,
) {
    let actual = validate_return_to(return_to, control_plane_url);
    match (actual, expected) {
        (Ok(valid), Ok(wanted)) => assert_eq!(valid, wanted),
        (Err(SsoReturnTargetError::OriginMismatch), Err(())) => {}
        (outcome, _) => panic!("unexpected {outcome:?}"),
    }
}

#[rstest]
#[case::return_to_port("https://cp.example.com:notaport/x", CONTROL_PLANE)]
#[case::return_to_range("https://cp.example.com:70000/x", CONTROL_PLANE)]
#[case::configured_port(CONTROL_PLANE, "https://cp.example.com:notaport")]
fn reports_a_malformed_port_instead_of_a_mismatch(
    #[case] return_to: &str,
    #[case] control_plane_url: &str,
) {
    let error = validate_return_to(return_to, Some(control_plane_url)).unwrap_err();
    assert!(matches!(error, SsoReturnTargetError::Parse(_)));
    assert_eq!(error.response().status, 400);
}

#[test]
fn builds_the_session_cookie_without_httponly() {
    assert_eq!(
        session_token_cookie(JWT, false).unwrap(),
        "token=jwt.token-value; Path=/; SameSite=lax"
    );
    assert_eq!(
        session_token_cookie(JWT, true).unwrap(),
        "token=jwt.token-value; Path=/; SameSite=lax; Secure"
    );
    let error = session_token_cookie("bad\ntoken", false).unwrap_err();
    assert!(matches!(error, SsoReturnTargetError::CookieValue));
    assert_eq!(error.response().status, 400);
}

#[rstest]
#[case::plain("dashboard", "dashboard")]
#[case::path("/dashboard?x=1", "\"/dashboard?x=1\"")]
#[case::semicolon("/a;b", "\"/a\\073b\"")]
#[case::quotes("/a\"b", "\"/a\\\"b\"")]
#[case::backslash("/a\\b", "\"/a\\\\b\"")]
#[case::unicode("/résumé", "\"/r\\351sum\\351\"")]
#[case::code_255("/\u{00ff}", "\"/\\377\"")]
#[case::above_255("/\u{20ac}", "\"/\u{20ac}\"")]
#[case::empty("", "\"\"")]
fn quotes_cookie_values_like_the_stdlib(#[case] value: &str, #[case] expected: &str) {
    let cookie = return_to_cookie(value).unwrap();
    assert_eq!(
        cookie,
        format!("{RETURN_TO_COOKIE}={expected}; HttpOnly; Max-Age=600; Path=/; SameSite=lax")
    );
}

#[test]
fn deletes_the_return_to_cookie_at_the_current_time() {
    assert_eq!(delete_return_to_cookie(NOW), DELETED);
    assert_eq!(
        delete_return_to_cookie(0),
        "litellm_cp_return_to=\"\"; expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=0; Path=/; SameSite=lax"
    );
}

#[test]
fn derives_secure_from_the_trusted_network() {
    assert!(!is_secure_request(
        &http_request(),
        &TrustedNetworkSettings::default()
    ));
    assert!(is_secure_request(
        &https_request(),
        &TrustedNetworkSettings::default()
    ));
    assert!(is_secure_request(&http_request(), &https_settings()));

    let trusted = TrustedNetworkSettings {
        proxy_base_url: String::new(),
        use_x_forwarded_for: true,
        trusted_proxy_ranges: None,
        mcp_trusted_proxy_ranges: Some(vec!["10.0.0.0/8".to_owned()]),
        mcp_xff_num_trusted_hops: None,
    };
    let mut forwarded = http_request();
    forwarded.headers_mut().insert(
        "x-forwarded-proto",
        header::HeaderValue::from_static("https"),
    );
    forwarded
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([10, 1, 2, 3], 5000))));
    assert!(is_secure_request(&forwarded, &trusted));
    assert!(!is_secure_request(
        &forwarded,
        &TrustedNetworkSettings::default()
    ));
}

#[test]
fn sets_the_token_cookie_on_a_response() {
    let mut response = Response::new(Body::empty());
    set_session_token_cookie(&mut response, &https_request(), &https_settings(), JWT).unwrap();
    assert_eq!(
        header_values(&response, header::SET_COOKIE),
        vec!["token=jwt.token-value; Path=/; SameSite=lax; Secure".to_owned()]
    );
}

#[test]
fn persists_only_safe_return_targets() {
    let request = https_request();
    let settings = https_settings();
    let persist = |return_to: Option<&str>, control_plane_url: Option<&str>| {
        let mut response = Response::new(Body::empty());
        persist_return_to_cookie(
            &mut response,
            return_to,
            control_plane_url,
            &request,
            &settings,
        )
        .unwrap();
        header_values(&response, header::SET_COOKIE)
    };

    assert!(persist(None, Some(CONTROL_PLANE)).is_empty());
    assert!(persist(Some("https://evil.example.com/x"), Some(CONTROL_PLANE)).is_empty());
    assert!(persist(Some("https://cp.example.com/x"), None).is_empty());
    assert!(persist(Some("https://cp.example.com/a\nb"), Some(CONTROL_PLANE)).is_empty());
    assert_eq!(
        persist(Some("/dashboard"), Some(CONTROL_PLANE)),
        vec![
            "litellm_cp_return_to=\"/dashboard\"; HttpOnly; Max-Age=600; Path=/; SameSite=lax; Secure"
                .to_owned()
        ]
    );
    assert_eq!(
        persist(Some("https://cp.example.com/next"), Some(CONTROL_PLANE)),
        vec![
            "litellm_cp_return_to=\"https://cp.example.com/next\"; HttpOnly; Max-Age=600; Path=/; SameSite=lax; Secure"
                .to_owned()
        ]
    );
}

#[test]
fn persist_reports_a_malformed_return_target() {
    let mut response = Response::new(Body::empty());
    let error = persist_return_to_cookie(
        &mut response,
        Some("https://cp.example.com:notaport"),
        Some(CONTROL_PLANE),
        &https_request(),
        &https_settings(),
    )
    .unwrap_err();
    assert!(matches!(error, SsoReturnTargetError::Parse(_)));
    assert!(header_values(&response, header::SET_COOKIE).is_empty());
}

#[test]
fn reports_entropy_failures_from_the_login_code() {
    let error = generate_login_code(&FailingEntropy).unwrap_err();
    assert!(matches!(error, SsoReturnTargetError::Entropy(_)));
    assert_eq!(error.response().status, 500);
}

#[rstest]
#[case::trailing_slash(
    "http://localhost:4000/",
    None,
    "",
    "sso/callback",
    "http://localhost:4000/sso/callback"
)]
#[case::no_trailing_slash(
    "http://localhost:4000",
    None,
    "",
    "sso/callback",
    "http://localhost:4000/sso/callback"
)]
#[case::bare_root("/", None, "", "sso/callback", "/sso/callback")]
#[case::proxy_base_url(
    "http://localhost:4000/",
    Some("https://cp.example.com"),
    "",
    "sso/callback",
    "https://cp.example.com/sso/callback"
)]
#[case::root_path(
    "http://localhost:4000/",
    None,
    "/litellm",
    "sso/callback",
    "http://localhost:4000/litellm/sso/callback"
)]
fn builds_the_sso_callback_url(
    #[case] base_url: &str,
    #[case] proxy_base_url: Option<&str>,
    #[case] root_path: &str,
    #[case] route: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        get_redirect_url_for_sso(
            proxy_base_url,
            base_url,
            root_path,
            route,
            Some("existing-key")
        ),
        expected
    );
}

#[tokio::test]
async fn redirects_a_relative_target_with_the_session_cookie() {
    let request = https_request();
    let settings = https_settings();
    let cache = RecordingCache::default();
    let response = sso_return_to_redirect(&redirect_to(
        Some("/dashboard?tab=1"),
        Some(CONTROL_PLANE),
        &request,
        &settings,
        login_caches(&cache, None),
    ))
    .await
    .unwrap()
    .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_values(&response, header::LOCATION),
        vec!["/dashboard?tab=1".to_owned()]
    );
    assert_eq!(
        header_values(&response, header::SET_COOKIE),
        vec![
            "token=jwt.token-value; Path=/; SameSite=lax; Secure".to_owned(),
            DELETED.to_owned(),
        ]
    );
    assert!(cache.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn redirects_a_cross_origin_target_behind_a_login_code() {
    let request = https_request();
    let settings = https_settings();
    let cache = RecordingCache::default();
    let response = sso_return_to_redirect(&redirect_to(
        Some("https://cp.example.com/done?x=1"),
        Some(CONTROL_PLANE),
        &request,
        &settings,
        login_caches(&cache, None),
    ))
    .await
    .unwrap()
    .unwrap();

    let recorded = cache.writes.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    let entry = &recorded[0];
    let code = entry.key.strip_prefix(LOGIN_CODE_PREFIX).unwrap();
    assert_eq!(code.len(), 43);
    assert!(
        code.chars()
            .all(|letter| letter.is_ascii_alphanumeric() || matches!(letter, '-' | '_'))
    );
    assert_eq!(entry.token, JWT);
    assert_eq!(entry.redirect_url, "https://cp.example.com/done?x=1");
    assert_eq!(entry.ttl_seconds, 60.0_f64);

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_values(&response, header::LOCATION),
        vec![format!(
            "https://cp.example.com/done?x=1&login=success&code={code}"
        )]
    );
    assert_eq!(
        header_values(&response, header::SET_COOKIE),
        vec![DELETED.to_owned()]
    );
}

#[tokio::test]
async fn prefers_the_redis_usage_cache_and_appends_with_a_question_mark() {
    let request = https_request();
    let settings = https_settings();
    let redis = RecordingCache::default();
    let user = RecordingCache::default();
    let response = sso_return_to_redirect(&redirect_to(
        Some("https://cp.example.com/done"),
        Some(CONTROL_PLANE),
        &request,
        &settings,
        login_caches(&user, Some(&redis)),
    ))
    .await
    .unwrap()
    .unwrap();

    assert_eq!(redis.writes.lock().unwrap().len(), 1);
    assert!(user.writes.lock().unwrap().is_empty());
    let found = header_values(&response, header::LOCATION);
    let location = found.first().unwrap();
    assert!(location.starts_with("https://cp.example.com/done?login=success&code="));
}

#[tokio::test]
async fn returns_none_for_missing_or_untrusted_targets() {
    let request = https_request();
    let settings = https_settings();
    let cache = RecordingCache::default();

    let missing = sso_return_to_redirect(&redirect_to(
        None,
        Some(CONTROL_PLANE),
        &request,
        &settings,
        login_caches(&cache, None),
    ))
    .await
    .unwrap();
    assert!(missing.is_none());

    let unconfigured = sso_return_to_redirect(&redirect_to(
        Some("https://cp.example.com/done"),
        None,
        &request,
        &settings,
        login_caches(&cache, None),
    ))
    .await
    .unwrap();
    assert!(unconfigured.is_none());

    let mismatch = sso_return_to_redirect(&redirect_to(
        Some("https://evil.example.com/done"),
        Some(CONTROL_PLANE),
        &request,
        &settings,
        login_caches(&cache, None),
    ))
    .await
    .unwrap_err();
    assert_eq!(mismatch.response().status, 400);
    assert!(cache.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reports_a_cache_write_failure() {
    let request = https_request();
    let settings = https_settings();
    let cache = RecordingCache::failing();
    let error = sso_return_to_redirect(&redirect_to(
        Some("https://cp.example.com/done"),
        Some(CONTROL_PLANE),
        &request,
        &settings,
        login_caches(&cache, None),
    ))
    .await
    .unwrap_err();
    assert!(matches!(error, SsoReturnTargetError::CacheWrite(_)));
    assert_eq!(error.response().status, 500);
}

#[tokio::test]
async fn stores_the_login_code_in_the_user_cache() {
    let cache = local_auth_cache();
    let stored = LoginCode {
        token: JWT,
        redirect_url: "https://cp.example.com/done",
    };
    cache
        .store_login_code("login_code:abc", &stored, 60.0_f64)
        .await
        .unwrap();
    let readback = cache.async_get("login_code:abc", false).await.unwrap();
    let Json::Object(fields) = readback else {
        panic!("the login code is stored as a JSON object");
    };
    assert_eq!(fields.get("token"), Some(&Json::Text(JWT.to_owned())));
    assert_eq!(
        fields.get("redirect_url"),
        Some(&Json::Text("https://cp.example.com/done".to_owned()))
    );
}

#[tokio::test]
async fn reports_a_redis_usage_cache_write_failure() {
    let cache = open_breaker_redis_cache().await;
    let stored = LoginCode {
        token: JWT,
        redirect_url: "https://cp.example.com/done",
    };
    let error = cache
        .store_login_code("login_code:abc", &stored, 60.0_f64)
        .await
        .unwrap_err();
    assert!(matches!(error, CacheWriteError::RedisUsage(_)));
}

#[rstest]
#[case::percent_encoded(b"return_to=%2Fdashboard", Some("/dashboard"))]
#[case::last_value_wins(b"return_to=%2Fa&return_to=%2Fb", Some("/b"))]
#[case::plus_is_space(b"return_to=%2Fa+b", Some("/a b"))]
#[case::absent(b"next=%2Fdashboard", None)]
#[case::empty(b"return_to=", Some(""))]
fn decodes_the_return_to_query_parameter(#[case] query: &[u8], #[case] expected: Option<&str>) {
    assert_eq!(decode_sso_return_target(query).as_deref(), expected);
}

#[rstest]
fn replays_the_sso_return_target_corpus(#[files("fuzz/corpus/sso_return_target/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Some(target) = decode_sso_return_target(&bytes) else {
        return;
    };
    assert!(!is_same_origin_return_path(&target) || target.starts_with('/'));
}
