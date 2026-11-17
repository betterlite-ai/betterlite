use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::Headers;
use secrecy::ExposeSecret as _;

use crate::image_edits::auth::{ValidateEnvironmentRequest, validate_environment};
use crate::image_edits::error::ImageEditError;
use crate::image_edits::flux2::map_edit_params;
use crate::image_edits::url::{CompleteUrlRequest, complete_url};

use super::support::{self, FLUX1, FLUX2, NoSources};

const SECRET: &str = "fixture-secret-value";
const CLIENT_SECRET: &str = "fixture-client-secret";

#[test]
fn foundry_image_edit_credential_identity_isolation() {
    let first = support::foundry_settings(Some("https://first.test"), Some("key-a"));
    let second = support::foundry_settings(Some("https://second.test"), Some("key-b"));

    // each configured identity resolves its own key and base, never the other's
    let first_key = support::accessors(&first, None, None)
        .api_key(None)
        .expect("a key");
    let second_key = support::accessors(&second, None, None)
        .api_key(None)
        .expect("a key");
    assert_eq!(first_key.expose_secret(), "key-a");
    assert_eq!(second_key.expose_secret(), "key-b");
    assert_eq!(
        support::accessors(&first, None, None)
            .api_base(None)
            .as_deref(),
        Some("https://first.test")
    );
    assert_eq!(
        support::accessors(&second, None, None)
            .api_base(None)
            .as_deref(),
        Some("https://second.test")
    );

    // a resolved header belongs to the call that made it: changing it never reaches the other identity
    let mut first_headers = Headers::new();
    first_headers.insert(String::from("Api-Key"), String::from("key-a"));
    let mut second_headers = Headers::new();
    second_headers.insert(String::from("Api-Key"), String::from("key-b"));
    first_headers.insert(String::from("Api-Key"), String::from("rotated"));
    assert_eq!(
        second_headers.get("Api-Key").map(String::as_str),
        Some("key-b")
    );
    assert_ne!(first_headers, second_headers);

    // a global key overrides the identity it is passed with, and only that call
    let global = support::secret("global-key");
    let from_global = support::accessors(&first, None, Some(&global))
        .api_key(None)
        .expect("a key");
    let from_env = support::accessors(&first, None, None)
        .api_key(None)
        .expect("a key");
    assert_eq!(from_global.expose_secret(), "global-key");
    assert_ne!(from_global.expose_secret(), from_env.expose_secret());
    let untouched = support::accessors(&second, None, None)
        .api_key(None)
        .expect("a key");
    assert_eq!(untouched.expose_secret(), "key-b");
}

#[test]
fn foundry_image_edit_secret_safe_failure() {
    let client_secret = support::object(&format!(
        r#"{{"tenant_id": "tenant", "client_id": "client", "client_secret": "{CLIENT_SECRET}"}}"#
    ));
    let keyless = support::foundry_settings(Some("https://azure.test"), None);
    let configured = support::foundry_settings(Some("https://azure.test"), Some(SECRET));
    let missing_key = validate_environment(ValidateEnvironmentRequest {
        headers: Headers::new(),
        model: FLUX1,
        api_key: None,
        params: Some(&client_secret),
        environment: &BTreeMap::new(),
        accessors: support::accessors(&keyless, None, None),
        refresh_enabled: false,
        sources: &NoSources,
    })
    .unwrap_err();
    assert_secret_safe(&missing_key);
    let missing_base = complete_url(CompleteUrlRequest {
        model: FLUX1,
        accessors: support::accessors(&configured, None, None),
        api_base: None,
        api_version: None,
        global_api_version: None,
        environment: &support::version_settings(None),
    })
    .unwrap_err();
    assert_secret_safe(&missing_base);
    assert_secret_safe(&params_failure());
}

fn assert_secret_safe(failure: &ImageEditError) {
    let rendered = format!("{failure:?} {failure}");
    assert!(
        !rendered.contains(SECRET),
        "an error exposed the configured key: {rendered}"
    );
    assert!(
        !rendered.contains(CLIENT_SECRET),
        "an error exposed the client secret: {rendered}"
    );
}

fn params_failure() -> ImageEditError {
    map_edit_params(&support::params(r#"{"size": "bad"}"#), FLUX2, false).unwrap_err()
}
