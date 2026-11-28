use std::sync::atomic::Ordering;

use betterlite_config::sections::jwt_settings::Settings as JwtAuthSettings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use sha2::{Digest as _, Sha256};

use super::support::{
    Events, FakeReply, Harness, TestCache, TestHttp, capture_logs, harness, harness_with,
    jwt_settings_with_userinfo, lock, settings,
};

const ENDPOINT: &str = "https://idp.example.com/userinfo";

fn userinfo_harness(endpoint: &str, cache: TestCache, http: TestHttp) -> Harness {
    let jwt = jwt_settings_with_userinfo(Some(endpoint), 300.0);
    harness_with(settings(None, None, jwt), cache, http)
}

fn expected_key(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    format!("oidc_userinfo_{}", hex::encode(digest))
}

#[tokio::test]
async fn userinfo_endpoint_before_cache() {
    let none_configured = harness(settings(None, None, JwtAuthSettings::default()));
    let missing = none_configured
        .verifier
        .get_oidc_userinfo("token")
        .await
        .unwrap_err();
    assert_eq!(
        missing.to_string(),
        "OIDC UserInfo endpoint not configured. Set 'oidc_userinfo_endpoint' in JWT auth config."
    );
    assert!(none_configured.cache.events.take().is_empty());
    assert!(none_configured.http.requests().is_empty());

    let empty_configured = userinfo_harness("", TestCache::default(), TestHttp::default());
    let empty = empty_configured
        .verifier
        .get_oidc_userinfo("token")
        .await
        .unwrap_err();
    assert_eq!(
        empty.to_string(),
        "OIDC UserInfo endpoint not configured. Set 'oidc_userinfo_endpoint' in JWT auth config."
    );
    assert!(empty_configured.cache.events.take().is_empty());
    assert!(empty_configured.http.requests().is_empty());
}

#[tokio::test]
async fn userinfo_hashed_key_empty_hit_and_headers() {
    let token = "abc";
    let key = expected_key(token);
    assert!(key.starts_with("oidc_userinfo_"));
    let digest = key.trim_start_matches("oidc_userinfo_");
    assert_eq!(digest.len(), 64);
    assert!(
        digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    );
    assert_ne!(key, format!("oidc_userinfo_{token}"));

    let hit = userinfo_harness(
        ENDPOINT,
        TestCache::with_entry(&key, Json::Object(IndexMap::new())),
        TestHttp::default(),
    );
    let hit_userinfo = hit.verifier.get_oidc_userinfo(token).await.unwrap();
    assert!(hit_userinfo.is_empty());
    assert_eq!(hit.cache.events.take(), vec![format!("read:{key}")]);
    assert!(hit.http.requests().is_empty());

    let miss = userinfo_harness(
        ENDPOINT,
        TestCache::default(),
        TestHttp::with_replies(vec![FakeReply::Answer {
            status: 200,
            body: "{\"sub\":\"u-1\"}".to_owned(),
        }]),
    );
    let miss_userinfo = miss.verifier.get_oidc_userinfo(token).await.unwrap();
    assert!(matches!(miss_userinfo.get("sub"), Some(Json::Text(text)) if text == "u-1"));
    let requests = miss.http.requests();
    assert_eq!(requests.len(), 1);
    let request = requests.first().unwrap();
    assert_eq!(request.url, ENDPOINT);
    assert_eq!(request.authorization, "Bearer abc");
    assert_eq!(request.accept, "application/json");
    assert_eq!(
        miss.cache.events.take(),
        vec![format!("read:{key}"), format!("write:{key}")]
    );
}

#[tokio::test]
async fn userinfo_cached_non_object_is_a_miss() {
    let token = "abc";
    let key = expected_key(token);
    let stale = userinfo_harness(
        ENDPOINT,
        TestCache::with_entry(&key, Json::Text("not an identity document".to_owned())),
        TestHttp::with_replies(vec![FakeReply::Answer {
            status: 200,
            body: "{\"sub\":\"u-1\"}".to_owned(),
        }]),
    );
    let userinfo = stale.verifier.get_oidc_userinfo(token).await.unwrap();
    assert!(matches!(userinfo.get("sub"), Some(Json::Text(text)) if text == "u-1"));
    assert_eq!(stale.http.requests().len(), 1);
    assert_eq!(
        stale.cache.events.take(),
        vec![format!("read:{key}"), format!("write:{key}")]
    );
}

async fn status_error_without_retry() {
    let status_http = TestHttp::with_replies(vec![
        FakeReply::Answer {
            status: 500,
            body: "boom".to_owned(),
        },
        FakeReply::Answer {
            status: 200,
            body: "{}".to_owned(),
        },
    ]);
    let status_case = userinfo_harness(ENDPOINT, TestCache::default(), status_http);
    let status_error = status_case
        .verifier
        .get_oidc_userinfo("abc")
        .await
        .unwrap_err();
    assert_eq!(
        status_error.to_string(),
        "Failed to fetch OIDC UserInfo: OIDC UserInfo endpoint returned status 500: boom"
    );
    assert_eq!(status_case.http.requests().len(), 1);
}

async fn json_error_wrapped_as_fetch_failure() {
    let json_http = TestHttp::with_replies(vec![FakeReply::Answer {
        status: 200,
        body: "not-json".to_owned(),
    }]);
    let json_case = userinfo_harness(ENDPOINT, TestCache::default(), json_http);
    let json_error = json_case
        .verifier
        .get_oidc_userinfo("abc")
        .await
        .unwrap_err();
    assert_eq!(
        json_error.to_string(),
        "Failed to fetch OIDC UserInfo: the json document is not valid json"
    );
    assert_eq!(json_case.http.requests().len(), 1);
}

async fn write_error_wrapped_as_fetch_failure() {
    let write_http = TestHttp::with_replies(vec![FakeReply::Answer {
        status: 200,
        body: "{\"sub\":\"u\"}".to_owned(),
    }]);
    let write_case = userinfo_harness(ENDPOINT, TestCache::failing_write(), write_http);
    let write_error = write_case
        .verifier
        .get_oidc_userinfo("abc")
        .await
        .unwrap_err();
    assert_eq!(
        write_error.to_string(),
        "Failed to fetch OIDC UserInfo: the cache backend failed"
    );
    assert_eq!(write_case.http.requests().len(), 1);
}

#[tokio::test]
async fn userinfo_no_retry_status_json_write_error_wrapper() {
    status_error_without_retry().await;
    json_error_wrapped_as_fetch_failure().await;
    write_error_wrapped_as_fetch_failure().await;
}

#[tokio::test]
async fn userinfo_cache_read_error_unwrapped() {
    let harness = userinfo_harness(ENDPOINT, TestCache::failing_read(), TestHttp::default());
    let error = harness.verifier.get_oidc_userinfo("abc").await.unwrap_err();
    assert_eq!(error.to_string(), "the cache backend failed");
    assert!(!error.to_string().contains("Failed to fetch"));
    assert!(harness.http.requests().is_empty());
}

#[tokio::test]
async fn userinfo_cache_before_return_and_ttl() {
    let events = Events::default();
    let cache = TestCache {
        events: events.clone(),
        ..TestCache::default()
    };
    let http = TestHttp {
        events: events.clone(),
        ..TestHttp::with_replies(vec![FakeReply::Answer {
            status: 200,
            body: "{\"sub\":\"u\",\"email\":\"e\"}".to_owned(),
        }])
    };
    let jwt = jwt_settings_with_userinfo(Some(ENDPOINT), 300.0);
    let harness = harness_with(settings(None, None, jwt), cache, http);
    let token = "token";
    let userinfo = harness.verifier.get_oidc_userinfo(token).await.unwrap();
    assert!(matches!(userinfo.get("sub"), Some(Json::Text(text)) if text == "u"));
    let key = expected_key(token);
    assert_eq!(
        events.take(),
        vec![
            format!("read:{key}"),
            format!("http:{ENDPOINT}"),
            format!("write:{key}")
        ]
    );
    let writes = lock(&harness.cache.writes);
    assert_eq!(writes.len(), 1);
    let record = writes.first().unwrap();
    assert_eq!(record.key, key);
    assert!(record.ttl.total_cmp(&300.0_f64).is_eq());
    assert!(matches!(&record.value, Json::Object(fields) if fields.len() == 2));
}

#[tokio::test]
async fn close_http_handler() {
    let harness = harness(settings(None, None, JwtAuthSettings::default()));
    assert!(!harness.http.closed.load(Ordering::Acquire));
    drop(harness.verifier);
    assert!(harness.http.closed.load(Ordering::Acquire));
}

#[tokio::test]
async fn userinfo_failure() {
    let (logs, _guard) = capture_logs();
    let http = TestHttp::with_replies(vec![FakeReply::Transport]);
    let harness = userinfo_harness(ENDPOINT, TestCache::default(), http);
    let error = harness
        .verifier
        .get_oidc_userinfo("secret-token")
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Failed to fetch OIDC UserInfo: the identity provider could not be reached"
    );
    let lines = logs.take();
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("ERROR") && line.contains("Error fetching OIDC UserInfo"))
    );
    assert_eq!(harness.http.requests().len(), 1);
}

#[tokio::test]
async fn userinfo_logs_exclude_response_values() {
    let (logs, _guard) = capture_logs();
    let body = "{\"sub\":\"alice\",\"email\":\"alice@example.com\",\"access_token\":\"super-secret-value\"}";
    let http = TestHttp::with_replies(vec![FakeReply::Answer {
        status: 200,
        body: body.to_owned(),
    }]);
    let harness = userinfo_harness(ENDPOINT, TestCache::default(), http);
    let userinfo = harness
        .verifier
        .get_oidc_userinfo("opaque-token-value")
        .await
        .unwrap();
    assert!(matches!(
        userinfo.get("email"),
        Some(Json::Text(text)) if text == "alice@example.com"
    ));
    let lines = logs.take();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Received OIDC UserInfo"))
    );
    assert!(!lines.iter().any(|line| line.contains("alice")
        || line.contains("super-secret-value")
        || line.contains("opaque-token-value")));
}
