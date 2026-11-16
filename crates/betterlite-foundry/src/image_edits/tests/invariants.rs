use std::collections::BTreeMap;

use betterlite_config::sections::foundry_credentials::Settings as FoundrySettings;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::model_info::ModelInfoAccessors;
use crate::image_edits::auth::{ValidateEnvironmentRequest, validate_environment};
use crate::image_edits::error::ImageEditError;
use crate::image_edits::flux2::map_edit_params;
use crate::image_edits::url::{CompleteUrlRequest, complete_url};

use super::support::{self, FLUX1, FLUX2, NoSources};

const SECRET: &str = "fixture-secret-value";
const CLIENT_SECRET: &str = "fixture-client-secret";

fn identity<'data>(
    settings: &'data FoundrySettings,
    global_key: Option<&'data SecretString>,
) -> ModelInfoAccessors<'data> {
    ModelInfoAccessors {
        global_api_base: None,
        global_api_key: global_key,
        environment_api_base: settings.api_base.as_deref(),
        environment_api_key: settings.api_key.as_ref(),
    }
}

#[test]
fn foundry_image_edit_credential_identity_isolation() {
    let first = support::foundry_settings(Some("https://first.test"), Some("key-a"));
    let second = support::foundry_settings(Some("https://second.test"), Some("key-b"));

    // each configured identity resolves its own key and base, never the other's
    let first_key = identity(&first, None).api_key(None).expect("a key");
    let second_key = identity(&second, None).api_key(None).expect("a key");
    assert_eq!(first_key.expose_secret(), "key-a");
    assert_eq!(second_key.expose_secret(), "key-b");
    assert_eq!(
        identity(&first, None).api_base(None).as_deref(),
        Some("https://first.test")
    );
    assert_eq!(
        identity(&second, None).api_base(None).as_deref(),
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
    let from_global = identity(&first, Some(&global))
        .api_key(None)
        .expect("a key");
    let from_env = identity(&first, None).api_key(None).expect("a key");
    assert_ne!(from_global, from_env);
    assert_eq!(from_global.expose_secret(), "global-key");
    let untouched = identity(&second, None).api_key(None).expect("a key");
    assert_eq!(untouched.expose_secret(), "key-b");
}

#[test]
fn foundry_image_edit_secret_safe_failure() {
    let configured = support::foundry_settings(Some("https://azure.test"), Some(SECRET));
    let client_secret = support::object(&format!(
        r#"{{"tenant_id": "tenant", "client_id": "client", "client_secret": "{CLIENT_SECRET}"}}"#
    ));
    let keyless = support::foundry_settings(Some("https://azure.test"), None);
    let failures = [
        validate_failure(&keyless, &client_secret),
        url_failure(&configured),
        params_failure(),
    ];
    for failure in &failures {
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
}

fn validate_failure(settings: &FoundrySettings, params: &Object) -> ImageEditError {
    validate_environment(ValidateEnvironmentRequest {
        headers: Headers::new(),
        model: FLUX1,
        api_key: None,
        params: Some(params),
        environment: &BTreeMap::new(),
        accessors: identity(settings, None),
        refresh_enabled: false,
        sources: &NoSources,
    })
    .unwrap_err()
}

fn url_failure(settings: &FoundrySettings) -> ImageEditError {
    complete_url(&CompleteUrlRequest {
        model: FLUX1,
        accessors: identity(settings, None),
        api_base: None,
        api_version: None,
        global_api_version: None,
        environment: &support::version_settings(None),
    })
    .unwrap_err()
}

fn params_failure() -> ImageEditError {
    map_edit_params(&support::params(r#"{"size": "bad"}"#), FLUX2, false).unwrap_err()
}
