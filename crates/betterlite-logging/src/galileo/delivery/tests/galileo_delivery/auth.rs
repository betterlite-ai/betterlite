use super::super::super::auth::ensure_headers;
use super::super::super::health::check;
use super::{
    API_KEY_ENV, ScriptedTransport, TOKEN_ENV, cached_header_names, capture_logs, delivery,
    response, unreachable_host,
};

fn pair(name: &str, value: &str) -> (String, String) {
    (name.to_owned(), value.to_owned())
}

#[test]
fn an_api_key_sets_the_galileo_api_key_header_without_a_request() {
    let delivery = delivery(API_KEY_ENV);
    let transport = ScriptedTransport::default();
    assert!(ensure_headers(&delivery, &transport));
    assert!(transport.calls().is_empty());
    assert_eq!(
        cached_header_names(&delivery),
        Some(vec![
            "accept".to_owned(),
            "Content-Type".to_owned(),
            "Galileo-API-Key".to_owned()
        ])
    );
}

#[test]
fn username_and_password_log_in_with_a_form_post_and_cache_a_bearer_token() {
    let delivery = delivery(TOKEN_ENV);
    let transport =
        ScriptedTransport::replying(vec![Ok(response(200, r#"{"access_token": "tok-123"}"#))]);
    assert!(ensure_headers(&delivery, &transport));
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "POST");
    assert_eq!(calls[0].url, "https://galileo.example/login");
    assert_eq!(
        calls[0].headers,
        vec![
            pair("accept", "application/json"),
            pair("Content-Type", "application/x-www-form-urlencoded"),
        ]
    );
    assert_eq!(
        calls[0].form,
        vec![pair("username", "user"), pair("password", "pass")]
    );
    assert_eq!(
        cached_header_names(&delivery),
        Some(vec![
            "accept".to_owned(),
            "Content-Type".to_owned(),
            "Authorization".to_owned()
        ])
    );
}

#[test]
fn the_cached_bearer_header_carries_the_access_token() {
    let delivery = delivery(TOKEN_ENV);
    let transport = ScriptedTransport::replying(vec![
        Ok(response(200, r#"{"access_token": "tok-123"}"#)),
        Ok(response(200, "{}")),
    ]);
    assert!(ensure_headers(&delivery, &transport));
    let health = check(&delivery, &transport);
    assert_eq!(health.error_message, None);
    let calls = transport.calls();
    assert!(
        calls[1]
            .headers
            .contains(&pair("Authorization", "Bearer tok-123"))
    );
}

#[test]
fn cached_headers_are_returned_without_a_second_login() {
    let delivery = delivery(TOKEN_ENV);
    let transport =
        ScriptedTransport::replying(vec![Ok(response(200, r#"{"access_token": "tok-123"}"#))]);
    assert!(ensure_headers(&delivery, &transport));
    assert!(ensure_headers(&delivery, &transport));
    assert_eq!(transport.calls().len(), 1);
}

#[test]
fn a_rejected_login_returns_false_and_logs_at_debug() {
    let (logs, _guard) = capture_logs();
    let delivery = delivery(TOKEN_ENV);
    let transport = ScriptedTransport::replying(vec![Ok(response(401, "{}"))]);
    assert!(!ensure_headers(&delivery, &transport));
    assert_eq!(cached_header_names(&delivery), None);
    let text = logs.text();
    assert!(text.contains("DEBUG"), "{text}");
    assert!(text.contains("failed to authenticate"), "{text}");
}

#[test]
fn a_login_without_an_access_token_returns_false() {
    let delivery = delivery(TOKEN_ENV);
    let transport = ScriptedTransport::replying(vec![Ok(response(200, r#"{"token": "x"}"#))]);
    assert!(!ensure_headers(&delivery, &transport));
    assert_eq!(cached_header_names(&delivery), None);
}

#[test]
fn an_unreachable_login_returns_false() {
    let delivery = delivery(TOKEN_ENV);
    let transport = ScriptedTransport::replying(vec![unreachable_host()]);
    assert!(!ensure_headers(&delivery, &transport));
}

#[test]
fn token_auth_without_a_base_url_never_attempts_a_login() {
    let delivery = delivery(&[("GALILEO_USERNAME", "user"), ("GALILEO_PASSWORD", "pass")]);
    let transport = ScriptedTransport::default();
    assert!(!ensure_headers(&delivery, &transport));
    assert!(transport.calls().is_empty());
}

#[test]
fn no_credentials_means_no_headers() {
    let delivery = delivery(&[("GALILEO_PROJECT_ID", "project-1")]);
    assert!(!ensure_headers(&delivery, &ScriptedTransport::default()));
}
