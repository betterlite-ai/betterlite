#![cfg(test)]

use crate::credentials::{
    AuthValue, CredentialError, McpAuthType, McpClient, Request, ResolvedAuth, strip_auth_scheme,
    strip_header_whitespace, to_basic_auth, to_basic_credentials,
};
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one named proof combines the credential contract"
)]
fn credentials_tests_behavior() {
    assert_eq!(to_basic_auth("user:p\u{e4}ss"), "dXNlcjpww6Rzcw==");
    assert_eq!(strip_auth_scheme("bEaReR token", "Bearer"), "token");
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
    let mut extra = BTreeMap::new();
    extra.insert("authorization".to_owned(), "injected".to_owned());
    extra.insert(" X-Trace ".to_owned(), " value ".to_owned());
    let client = McpClient::new(
        "https://mcp.example".to_owned(),
        McpAuthType::BearerToken,
        Some(AuthValue::Text("Bearer token".to_owned())),
        Some("Authorization".to_owned()),
        Some(extra),
        None,
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
    let mut values = BTreeMap::new();
    values.insert("X-API-Key".to_owned(), "dict-key".to_owned());
    let dictionary_client = McpClient::new(
        "https://mcp.example".to_owned(),
        McpAuthType::ApiKey,
        Some(AuthValue::Headers(values)),
        None,
        None,
        None,
        None,
        None,
    );
    assert_eq!(
        dictionary_client.auth_headers().get("X-API-Key"),
        Some(&"dict-key".to_owned())
    );
    let auth = TestAuth {
        close_count: Arc::new(AtomicUsize::new(0)),
    };
    let closed = Arc::clone(&auth.close_count);
    let resolved_client = McpClient::new(
        "https://mcp.example".to_owned(),
        McpAuthType::None,
        None,
        None,
        None,
        None,
        Some(Box::new(auth)),
        None,
    );
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
    let mut whitespace = BTreeMap::new();
    whitespace.insert(" key ".to_owned(), " value ".to_owned());
    assert_eq!(
        strip_header_whitespace(whitespace).get("key"),
        Some(&"value".to_owned())
    );
}

struct TestAuth {
    close_count: Arc<AtomicUsize>,
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
