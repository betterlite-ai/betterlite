use std::collections::BTreeMap;

use betterlite_core::api::request_principal_metadata::{Metadata, PrincipalFacts};
use betterlite_core::api::request_setup_options::{
    CallbackVarsDecryption, RequestSetupOptions, RequestSetupSettings,
};
use betterlite_models::decisions::Json;

use crate::passthrough::callbacks::{
    TRUSTED_CALLBACK_VARS_FIELD, TeamCallbackWiring, resolve_team_callback_wiring,
};

struct Plaintext;

impl CallbackVarsDecryption for Plaintext {
    fn decrypt(&self, metadata: &Metadata) -> Metadata {
        metadata.clone()
    }
}

type Service = RequestSetupOptions<(), Plaintext, BTreeMap<String, String>>;

fn service() -> Service {
    RequestSetupOptions {
        settings: RequestSetupSettings {
            callback_disabling: false,
            missing_session_id: None,
            otel_v2_enabled: false,
            default_team_settings: Vec::new(),
            allowed_hosts: Vec::new(),
        },
        session_ids: (),
        decryption: Plaintext,
        environment: BTreeMap::new(),
    }
}

fn principal(text: &str) -> PrincipalFacts {
    serde_json::from_str(text).unwrap()
}

fn wiring(text: &str) -> TeamCallbackWiring {
    resolve_team_callback_wiring(&service(), &principal(text), "test route")
}

fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|name| (*name).to_owned()).collect()
}

fn assert_key_level() {
    let key_level = wiring(
        r#"{"metadata": {"logging": [{"callback_name": "langfuse", "callback_vars": {"langfuse_host": "https://key"}}]}}"#,
    );
    assert_eq!(key_level.success_callbacks, Some(names(&["langfuse"])));
    assert_eq!(key_level.failure_callbacks, Some(names(&["langfuse"])));
    let kwargs = key_level.logging_kwargs.unwrap();
    assert_eq!(
        kwargs["langfuse_host"],
        Json::Text("https://key".to_owned())
    );
    let Json::Object(trusted) = &kwargs[TRUSTED_CALLBACK_VARS_FIELD] else {
        panic!("trusted callback vars is not an object");
    };
    assert_eq!(
        trusted["langfuse_host"],
        Json::Text("https://key".to_owned())
    );
    assert_eq!(kwargs["metadata"], Json::Object(Metadata::new()));
    assert_eq!(kwargs["model_info"], Json::Object(Metadata::new()));
}

fn assert_without_vars_and_unconfigured() {
    // no callback vars leaves logging_kwargs unset but keeps the callback lists
    let without_vars = wiring(r#"{"metadata": {"logging": [{"callback_name": "langfuse"}]}}"#);
    assert_eq!(without_vars.success_callbacks, Some(names(&["langfuse"])));
    assert!(without_vars.logging_kwargs.is_none());

    let unconfigured = wiring("{}");
    assert_eq!(unconfigured, TeamCallbackWiring::default());
}

fn assert_fail_open() {
    // the deprecated callback_settings branch does not validate values, so an env reference fails open
    let env_reference = wiring(
        r#"{"team_metadata": {"callback_settings": {"callback_vars": {"langfuse_host": "os.environ/SECRET"}}}}"#,
    );
    assert_eq!(env_reference, TeamCallbackWiring::default());

    // a callback_settings value of the wrong shape fails open instead of failing the request
    let malformed =
        wiring(r#"{"team_metadata": {"callback_settings": {"success_callback": "not-a-list"}}}"#);
    assert_eq!(malformed, TeamCallbackWiring::default());
}

fn assert_deprecated() {
    let deprecated = wiring(
        r#"{"team_metadata": {"callback_settings": {"success_callback": ["langfuse"], "callback_vars": {"langfuse_host": "https://team"}}}}"#,
    );
    assert_eq!(deprecated.success_callbacks, Some(names(&["langfuse"])));
    assert_eq!(deprecated.failure_callbacks, Some(Vec::new()));
    assert_eq!(
        deprecated.logging_kwargs.unwrap()["langfuse_host"],
        Json::Text("https://team".to_owned())
    );
}

#[test]
fn team_callback_wiring() {
    let empty = TeamCallbackWiring::default();
    assert!(empty.success_callbacks.is_none());
    assert!(empty.failure_callbacks.is_none());
    assert!(empty.logging_kwargs.is_none());

    // Logging.__init__ copies these values out of the kwargs and may overwrite them
    let mut resolved = wiring(r#"{"metadata": {"logging": [{"callback_name": "langfuse"}]}}"#);
    resolved.success_callbacks = Some(names(&["datadog"]));
    assert_eq!(resolved.success_callbacks, Some(names(&["datadog"])));
    assert_eq!(resolved.failure_callbacks, Some(names(&["langfuse"])));
}

#[test]
fn passthrough_resolve_team_callback() {
    assert_key_level();
    assert_without_vars_and_unconfigured();
    assert_fail_open();
    assert_deprecated();
}
