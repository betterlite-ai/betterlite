#![cfg(test)]

use crate::credentials::{
    AuthValue, CredentialError, McpAuthType, McpClient, Request, ResolvedAuth, strip_auth_scheme,
    strip_header_whitespace, to_basic_auth, to_basic_credentials,
};
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

#[test]
fn credentials_tests_behavior() {
    assert_basic_credential_behavior();
    assert_client_header_behavior();
    assert_slot_and_empty_behavior();
}

fn assert_basic_credential_behavior() {
    assert_eq!(to_basic_auth("user:p\u{e4}ss"), "dXNlcjpww6Rzcw==");
    assert_eq!(strip_auth_scheme("bEaReR token", "Bearer"), "token");
    assert_eq!(
        strip_auth_scheme("Bearer token with spaces", "Bearer"),
        "token with spaces"
    );
    assert_eq!(
        strip_auth_scheme("Basic user:pass word", "Basic"),
        "user:pass word"
    );
    assert_eq!(strip_auth_scheme("Bearer", "Bearer"), "Bearer");
    assert_eq!(to_basic_credentials("Basic dXNlcjpwYXNz"), "dXNlcjpwYXNz");
    assert_eq!(
        to_basic_credentials("user:password"),
        "dXNlcjpwYXNzd29yZA=="
    );
    assert_eq!(
        to_basic_credentials("Basic user:password"),
        "dXNlcjpwYXNzd29yZA=="
    );
}

fn assert_client_header_behavior() {
    let mut extra = BTreeMap::new();
    extra.insert("authorization".to_owned(), "injected".to_owned());
    extra.insert(" X-Trace ".to_owned(), " value ".to_owned());
    let client = McpClient::new(
        "https://mcp.example".to_owned(),
        McpAuthType::BearerToken,
        Some(AuthValue::Text(SecretString::from("Bearer token"))),
        Some("Authorization".to_owned()),
        extra,
        Duration::from_secs(60),
        None,
        None,
    );
    let headers = client.auth_headers();
    assert_eq!(
        headers.get("Authorization"),
        Some(&"Bearer token".to_owned())
    );
    assert_eq!(headers.get("X-Trace"), Some(&"value".to_owned()));
    assert!(!headers.contains_key("authorization"));
    assert_dictionary_auth();
    assert_resolved_auth_behavior();
    let mut whitespace = BTreeMap::new();
    whitespace.insert(" key ".to_owned(), " value ".to_owned());
    assert_eq!(
        strip_header_whitespace(whitespace).get("key"),
        Some(&"value".to_owned())
    );
}

fn assert_dictionary_auth() {
    let mut values = BTreeMap::new();
    values.insert("X-API-Key".to_owned(), SecretString::from("dict-key"));
    let dictionary_client = McpClient::new(
        "https://mcp.example".to_owned(),
        McpAuthType::ApiKey,
        Some(AuthValue::Headers(values)),
        None,
        BTreeMap::new(),
        Duration::from_secs(60),
        None,
        None,
    );
    assert_eq!(
        dictionary_client.auth_headers().get("X-API-Key"),
        Some(&"dict-key".to_owned())
    );
}

fn assert_resolved_auth_behavior() {
    let auth = TestAuth {
        close_count: Arc::new(AtomicUsize::new(0)),
    };
    let closed = Arc::clone(&auth.close_count);
    let resolved_client = McpClient::new(
        "https://mcp.example".to_owned(),
        McpAuthType::None,
        None,
        None,
        BTreeMap::new(),
        Duration::from_secs(12),
        Some(Box::new(auth)),
        None,
    );
    assert_eq!(resolved_client.timeout, Duration::from_secs(12));
    let request = resolved_client
        .prepare_request_auth()
        .expect("auth flow should succeed");
    assert_eq!(request.headers.get("X-Resolved"), Some(&"yes".to_owned()));
    assert_eq!(closed.load(Ordering::SeqCst), 1);
    assert_eq!(
        resolved_client
            .discovery_auth_fingerprint()
            .expect("fingerprint"),
        resolved_client
            .discovery_auth_fingerprint()
            .expect("fingerprint")
    );
}

fn assert_slot_and_empty_behavior() {
    let resolved_slot_client = McpClient::new(
        "https://mcp.example".to_owned(),
        McpAuthType::BearerToken,
        Some(AuthValue::Text(SecretString::from("token"))),
        None,
        {
            let mut injected_headers = BTreeMap::new();
            injected_headers.insert("rEsOlVeD-aUtH".to_owned(), "injected".to_owned());
            injected_headers
        },
        Duration::from_secs(60),
        Some(Box::new(SlotAuth)),
        None,
    );
    assert_eq!(
        resolved_slot_client.auth_headers().get("resolved-auth"),
        Some(&"Bearer token".to_owned())
    );
    assert!(
        !resolved_slot_client
            .auth_headers()
            .contains_key("rEsOlVeD-aUtH")
    );

    let empty_client = McpClient::new(
        "https://mcp.example".to_owned(),
        McpAuthType::BearerToken,
        Some(AuthValue::Text(SecretString::from(""))),
        Some("Authorization".to_owned()),
        BTreeMap::new(),
        Duration::from_secs(60),
        None,
        None,
    );
    assert!(empty_client.auth_headers().is_empty());
}

struct TestAuth {
    close_count: Arc<AtomicUsize>,
}

struct SlotAuth;

impl ResolvedAuth for SlotAuth {
    fn authenticate(&self, request: Request) -> Result<Request, CredentialError> {
        Ok(request)
    }

    fn close(&self) {}

    fn header_name(&self) -> Option<&str> {
        Some("resolved-auth")
    }
}

impl ResolvedAuth for TestAuth {
    fn authenticate(&self, mut request: Request) -> Result<Request, CredentialError> {
        request
            .headers
            .insert("X-Resolved".to_owned(), "yes".to_owned());
        Ok(request)
    }

    fn close(&self) {
        self.close_count.fetch_add(1, Ordering::SeqCst);
    }
}
