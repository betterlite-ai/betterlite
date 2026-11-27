use super::support::{
    CountingEntropy, FailingCache, FailingEntropy, FixedEntropy, RecordingCache, SESSION_TOKEN,
    attribute, code_from_location, cookie_attributes, local_cache, resume_absolute,
    resume_relative, session_token, settings, user_only,
};
use crate::login_return::cache::LoginCaches;
use crate::login_return::redirect::{LoginReturnError, ResumeRequest, resume_login};
use crate::login_return::return_destination::ReturnDestinationError;
use betterlite_auth::trusted_network::request::NetworkRequest;
use betterlite_auth::trusted_network::settings::TrustedNetworkSettings;
use betterlite_models::decisions::Json;
use http::StatusCode;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
#[tokio::test]
async fn no_destination() {
    let token = session_token();
    let user = RecordingCache::default();
    let entropy = CountingEntropy::default();
    let result = resume_login(
        ResumeRequest {
            destination: None,
            session_token: &token,
            network: NetworkRequest::default(),
        },
        &settings(None),
        &TrustedNetworkSettings::default(),
        user_only(&user),
        &entropy,
    )
    .await
    .unwrap();
    assert!(result.is_none());
    assert!(user.writes().is_empty());
    assert_eq!(entropy.0.load(Ordering::Relaxed), 0);
}
#[tokio::test]
async fn relative_cookie_redirect() {
    let token = session_token();
    let user = RecordingCache::default();
    let network = NetworkRequest {
        scheme: "http",
        ..NetworkRequest::default()
    };
    let redirect = resume_relative(
        &token,
        b"/ui/settings?tab=keys",
        network,
        &TrustedNetworkSettings::default(),
        &user,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(redirect.status, StatusCode::SEE_OTHER);
    assert_eq!(redirect.location, "/ui/settings?tab=keys");
    assert_eq!(redirect.set_cookies.len(), 2);
    assert_eq!(
        redirect.set_cookies.first().map(String::as_str),
        Some("token=jwt.header.payload.signature; Path=/; SameSite=lax")
    );
    assert!(user.writes().is_empty());
}
#[tokio::test]
async fn absolute_code_redirect() {
    let token = session_token();
    let user = RecordingCache::default();
    let redirect = resume_absolute(
        &token,
        b"https://cp.example/console",
        "https://cp.example",
        &user,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(redirect.status, StatusCode::SEE_OTHER);
    let recorded = user.writes();
    assert_eq!(recorded.len(), 1);
    let stored = recorded.first().expect("one login code is written");
    let code = code_from_location(&redirect.location);
    assert_eq!(code.len(), 43);
    assert!(
        code.chars()
            .all(|character| character.is_ascii_alphanumeric()
                || character == '-'
                || character == '_')
    );
    assert_eq!(stored.key, format!("login_code:{code}"));
    assert_eq!(stored.token, SESSION_TOKEN);
    assert_eq!(stored.redirect_url, "https://cp.example/console");
    assert_eq!(stored.ttl_seconds.to_bits(), 60.0_f64.to_bits());
    assert_eq!(
        redirect.location,
        format!("https://cp.example/console?login=success&code={code}")
    );
    assert_eq!(redirect.set_cookies.len(), 1);
}
#[tokio::test]
async fn existing_query() {
    let token = session_token();
    let user = RecordingCache::default();
    let redirect = resume_absolute(
        &token,
        b"https://cp.example/console?tenant=a",
        "https://cp.example",
        &user,
    )
    .await
    .unwrap()
    .unwrap();
    let code = code_from_location(&redirect.location);
    assert_eq!(
        redirect.location,
        format!("https://cp.example/console?tenant=a&login=success&code={code}")
    );
}
#[tokio::test]
async fn fragment_concatenation() {
    let token = session_token();
    let user = RecordingCache::default();
    let redirect = resume_absolute(
        &token,
        b"https://cp.example/console#section",
        "https://cp.example",
        &user,
    )
    .await
    .unwrap()
    .unwrap();
    let code = code_from_location(&redirect.location);
    assert_eq!(
        redirect.location,
        format!("https://cp.example/console#section?login=success&code={code}")
    );
}
#[tokio::test]
async fn redis_before_response() {
    let token = session_token();
    let redis = RecordingCache::default();
    let user = RecordingCache::default();
    let redirect = resume_login(
        ResumeRequest {
            destination: Some(b"https://cp.example/console"),
            session_token: &token,
            network: NetworkRequest::default(),
        },
        &settings(Some("https://cp.example")),
        &TrustedNetworkSettings::default(),
        LoginCaches {
            redis: Some(&redis),
            user: &user,
        },
        &FixedEntropy,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(redirect.status, StatusCode::SEE_OTHER);
    assert_eq!(redis.writes().len(), 1);
    assert!(user.writes().is_empty());
}
#[tokio::test]
async fn local_cache_ttl() {
    let now = Arc::new(Mutex::new(0.0_f64));
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = {
        let now = Arc::clone(&now);
        Arc::new(move || *now.lock().unwrap_or_else(PoisonError::into_inner))
    };
    let cache = local_cache(clock);
    let token = session_token();
    let redirect = resume_login(
        ResumeRequest {
            destination: Some(b"https://cp.example/console"),
            session_token: &token,
            network: NetworkRequest::default(),
        },
        &settings(Some("https://cp.example")),
        &TrustedNetworkSettings::default(),
        user_only(&cache),
        &FixedEntropy,
    )
    .await
    .unwrap()
    .unwrap();
    let key = format!("login_code:{}", code_from_location(&redirect.location));
    let stored = cache
        .async_get(&key, true)
        .await
        .expect("the login code is stored");
    let Json::Object(fields) = stored else {
        panic!("the login code is stored as an object");
    };
    assert!(matches!(
        fields.get("token"),
        Some(Json::Text(text)) if text.as_str() == SESSION_TOKEN
    ));
    assert!(matches!(
        fields.get("redirect_url"),
        Some(Json::Text(text)) if text.as_str() == "https://cp.example/console"
    ));
    *now.lock().unwrap_or_else(PoisonError::into_inner) = 59.5_f64;
    assert!(cache.async_get(&key, true).await.is_some());
    *now.lock().unwrap_or_else(PoisonError::into_inner) = 60.5_f64;
    assert!(cache.async_get(&key, true).await.is_none());
}
#[tokio::test]
async fn failed_cache_no_redirect() {
    let token = session_token();
    let outcome = resume_absolute(
        &token,
        b"https://cp.example/console",
        "https://cp.example",
        &FailingCache,
    )
    .await;
    assert!(matches!(outcome, Err(LoginReturnError::Cache(_))));
}
#[tokio::test]
async fn unconfigured_absolute_is_ignored() {
    let token = session_token();
    let user = RecordingCache::default();
    let result = resume_login(
        ResumeRequest {
            destination: Some(b"https://cp.example/console"),
            session_token: &token,
            network: NetworkRequest::default(),
        },
        &settings(None),
        &TrustedNetworkSettings::default(),
        user_only(&user),
        &FixedEntropy,
    )
    .await
    .unwrap();
    assert!(result.is_none());
    assert!(user.writes().is_empty());
}
#[tokio::test]
async fn invalid_destination_bytes() {
    let token = session_token();
    let user = RecordingCache::default();
    let result = resume_login(
        ResumeRequest {
            destination: Some(b"/ui/\xff"),
            session_token: &token,
            network: NetworkRequest::default(),
        },
        &settings(None),
        &TrustedNetworkSettings::default(),
        user_only(&user),
        &FixedEntropy,
    )
    .await;
    assert!(matches!(
        result,
        Err(LoginReturnError::Destination(
            ReturnDestinationError::NotUtf8(_)
        ))
    ));
}
#[tokio::test]
async fn entropy_failure_propagates() {
    let token = session_token();
    let user = RecordingCache::default();
    let result = resume_login(
        ResumeRequest {
            destination: Some(b"https://cp.example/console"),
            session_token: &token,
            network: NetworkRequest::default(),
        },
        &settings(Some("https://cp.example")),
        &TrustedNetworkSettings::default(),
        user_only(&user),
        &FailingEntropy,
    )
    .await;
    assert!(matches!(result, Err(LoginReturnError::Entropy(_))));
    assert!(user.writes().is_empty());
}
#[tokio::test]
async fn location_quoting() {
    let token = session_token();
    let user = RecordingCache::default();
    let redirect = resume_relative(
        &token,
        "/café/%2F?q=<tag>".as_bytes(),
        NetworkRequest::default(),
        &TrustedNetworkSettings::default(),
        &user,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(redirect.location, "/caf%C3%A9/%2F?q=%3Ctag%3E");
}
#[tokio::test]
async fn one_shot_cookie_deletion() {
    let token = session_token();
    let user = RecordingCache::default();
    let redirect = resume_relative(
        &token,
        b"/ui/return",
        NetworkRequest::default(),
        &TrustedNetworkSettings::default(),
        &user,
    )
    .await
    .unwrap()
    .unwrap();
    let deletion = redirect
        .set_cookies
        .last()
        .expect("the relative arm clears the return cookie");
    let attributes = cookie_attributes(deletion);
    assert_eq!(attribute(&attributes, "litellm_cp_return_to"), Some("\"\""));
    assert!(attribute(&attributes, "expires").is_some());
    assert_eq!(attribute(&attributes, "Max-Age"), Some("0"));
    assert_eq!(attribute(&attributes, "Path"), Some("/"));
    assert_eq!(attribute(&attributes, "SameSite"), Some("lax"));
    assert_eq!(attribute(&attributes, "Secure"), None);
    assert_eq!(attribute(&attributes, "HttpOnly"), None);
}
