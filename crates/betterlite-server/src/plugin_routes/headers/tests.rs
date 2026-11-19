use axum::http::{HeaderMap, HeaderValue};
use secrecy::SecretString;

use super::{request_headers, response_headers};
use crate::plugin_routes::error::PluginError;

fn map(pairs: &[(&'static str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.insert(*name, HeaderValue::from_str(value).unwrap());
    }
    headers
}

#[test]
fn strips_hop_by_hop_credential_and_cookie_headers() {
    let raw = map(&[
        ("host", "plugin.local"),
        ("connection", "keep-alive"),
        ("transfer-encoding", "chunked"),
        ("te", "trailers"),
        ("trailers", "x-checksum"),
        ("upgrade", "websocket"),
        ("cookie", "session=abc"),
        ("authorization", "Bearer sk-caller"),
        ("api-key", "sk-caller"),
        ("x-api-key", "sk-caller"),
        ("x-goog-api-key", "sk-caller"),
        ("ocp-apim-subscription-key", "sk-caller"),
        ("x-litellm-api-key", "sk-caller"),
        ("x-keep", "yes"),
    ]);
    let forwarded = request_headers(&raw, None, None, None, None).unwrap();
    assert_eq!(forwarded.len(), 1);
    assert_eq!(forwarded["x-keep"], "yes");
}

#[test]
fn keeps_ordinary_headers() {
    let raw = map(&[
        ("accept", "text/html"),
        ("content-type", "application/json"),
    ]);
    let forwarded = request_headers(&raw, None, None, None, None).unwrap();
    assert_eq!(forwarded["accept"], "text/html");
    assert_eq!(forwarded["content-type"], "application/json");
}

#[test]
fn strips_the_configured_key_header_ignoring_case() {
    let raw = map(&[("x-custom-key", "sk-caller"), ("x-keep", "yes")]);
    let forwarded = request_headers(&raw, Some("X-Custom-Key"), None, None, None).unwrap();
    assert!(!forwarded.contains_key("x-custom-key"));
    assert_eq!(forwarded["x-keep"], "yes");
}

#[test]
fn injects_the_plugin_key_and_the_caller_identity() {
    let key = SecretString::from("sk-plugin");
    let forwarded = request_headers(
        &HeaderMap::new(),
        None,
        Some(&key),
        Some("u1"),
        Some("proxy_admin"),
    )
    .unwrap();
    assert_eq!(forwarded["authorization"], "Bearer sk-plugin");
    assert_eq!(forwarded["x-litellm-user-id"], "u1");
    assert_eq!(forwarded["x-litellm-user-role"], "proxy_admin");
}

#[test]
fn overrides_a_caller_authorization_with_the_plugin_key() {
    let raw = map(&[("authorization", "Bearer sk-caller")]);
    let key = SecretString::from("sk-plugin");
    let forwarded = request_headers(&raw, None, Some(&key), None, None).unwrap();
    assert_eq!(forwarded["authorization"], "Bearer sk-plugin");
}

#[test]
fn omits_absent_plugin_key_and_identity() {
    let raw = map(&[("authorization", "Bearer sk-caller")]);
    let forwarded = request_headers(&raw, None, None, None, None).unwrap();
    assert!(!forwarded.contains_key("authorization"));
    assert!(!forwarded.contains_key("x-litellm-user-id"));
    assert!(!forwarded.contains_key("x-litellm-user-role"));
}

#[test]
fn keeps_the_last_value_of_a_repeated_header() {
    let mut raw = HeaderMap::new();
    raw.append("x-multi", HeaderValue::from_static("first"));
    raw.append("x-multi", HeaderValue::from_static("second"));
    let forwarded = request_headers(&raw, None, None, None, None).unwrap();
    assert_eq!(forwarded["x-multi"], "second");
}

#[test]
fn rejects_a_plugin_key_with_a_newline() {
    let key = SecretString::from("sk\nplugin");
    let result = request_headers(&HeaderMap::new(), None, Some(&key), None, None);
    assert!(matches!(
        result,
        Err(PluginError::HeaderValue {
            name: "authorization"
        })
    ));
}

#[test]
fn rejects_a_user_id_with_a_newline() {
    let result = request_headers(&HeaderMap::new(), None, None, Some("u\n1"), None);
    assert!(matches!(
        result,
        Err(PluginError::HeaderValue {
            name: "x-litellm-user-id"
        })
    ));
}

#[test]
fn rejects_a_user_role_with_a_newline() {
    let result = request_headers(&HeaderMap::new(), None, None, None, Some("proxy\nadmin"));
    assert!(matches!(
        result,
        Err(PluginError::HeaderValue {
            name: "x-litellm-user-role"
        })
    ));
}

#[test]
fn response_drops_encoding_length_and_cookies() {
    let raw = map(&[
        ("content-encoding", "gzip"),
        ("transfer-encoding", "chunked"),
        ("content-length", "10"),
        ("set-cookie", "session=abc"),
        ("content-type", "text/html"),
        ("x-plugin", "1"),
    ]);
    let safe = response_headers(&raw);
    assert_eq!(safe.len(), 4);
    assert_eq!(safe["content-type"], "text/html");
    assert_eq!(safe["x-plugin"], "1");
    assert_eq!(safe["content-security-policy"], "sandbox");
    assert_eq!(safe["x-content-type-options"], "nosniff");
}

#[test]
fn response_forces_the_sandbox_headers_over_the_plugin_values() {
    let raw = map(&[
        ("content-security-policy", "default-src *"),
        ("x-content-type-options", "sniff"),
    ]);
    let safe = response_headers(&raw);
    assert_eq!(safe["content-security-policy"], "sandbox");
    assert_eq!(safe["x-content-type-options"], "nosniff");
}
