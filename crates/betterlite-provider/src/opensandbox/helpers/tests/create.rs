use std::collections::BTreeMap;

use rstest::rstest;
use serde_json::value::RawValue;

use super::super::create::{CreateSandbox, create_body, default_resource_limits};

fn request() -> CreateSandbox<'static> {
    CreateSandbox {
        template: None,
        timeout: None,
        allow_internet_access: None,
        metadata: None,
        env_vars: None,
        resource_limits: None,
        resource_requests: None,
        entrypoint: None,
        network_policy: None,
        secure_access: false,
    }
}

#[test]
fn create_body_fills_every_default() {
    let body = create_body(&request());
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"image":{"uri":"opensandbox/code-interpreter:v1.1.0"},"entrypoint":["/opt/code-interpreter/code-interpreter.sh"],"timeout":300,"resourceLimits":{"cpu":"1","memory":"2Gi"},"networkPolicy":{"defaultAction":"deny","egress":[]}}"#
    );
}

#[test]
fn create_body_copies_every_supplied_option() {
    let metadata = BTreeMap::from([("team".to_owned(), "data".to_owned())]);
    let env_vars = BTreeMap::from([("FOO".to_owned(), "bar".to_owned())]);
    let limits = BTreeMap::from([
        ("cpu".to_owned(), "2".to_owned()),
        ("memory".to_owned(), "4Gi".to_owned()),
    ]);
    let requests = BTreeMap::from([("cpu".to_owned(), "0.5".to_owned())]);
    let entrypoint = vec!["/bin/sh".to_owned(), "-c".to_owned()];
    let policy: Box<RawValue> = serde_json::from_str(
        r#"{"defaultAction":"allow","egress":[{"action":"allow","target":"example.com"}]}"#,
    )
    .unwrap();
    let body = create_body(&CreateSandbox {
        template: Some("opensandbox/custom:v2"),
        timeout: Some(120),
        allow_internet_access: Some(false),
        metadata: Some(&metadata),
        env_vars: Some(&env_vars),
        resource_limits: Some(&limits),
        resource_requests: Some(&requests),
        entrypoint: Some(&entrypoint),
        network_policy: Some(&policy),
        secure_access: true,
    });
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"image":{"uri":"opensandbox/custom:v2"},"entrypoint":["/bin/sh","-c"],"timeout":120,"resourceLimits":{"cpu":"2","memory":"4Gi"},"metadata":{"team":"data"},"env":{"FOO":"bar"},"resourceRequests":{"cpu":"0.5"},"networkPolicy":{"defaultAction":"allow","egress":[{"action":"allow","target":"example.com"}]},"secureAccess":true}"#
    );
}

#[rstest]
#[case::absent(None)]
#[case::false_value(Some(false))]
fn create_body_denies_egress_unless_internet_access_is_exactly_true(
    #[case] allow_internet_access: Option<bool>,
) {
    let body = create_body(&CreateSandbox {
        allow_internet_access,
        ..request()
    });
    assert_eq!(
        serde_json::to_string(&body.network_policy).unwrap(),
        r#"{"defaultAction":"deny","egress":[]}"#
    );
}

#[test]
fn create_body_omits_the_policy_when_internet_access_is_true() {
    let body = create_body(&CreateSandbox {
        allow_internet_access: Some(true),
        ..request()
    });
    assert!(body.network_policy.is_none());
}

#[test]
fn create_body_prefers_an_explicit_network_policy() {
    let policy: Box<RawValue> = serde_json::from_str(r#"{"defaultAction":"allow"}"#).unwrap();
    let body = create_body(&CreateSandbox {
        allow_internet_access: Some(true),
        network_policy: Some(&policy),
        ..request()
    });
    assert_eq!(
        serde_json::to_string(&body.network_policy).unwrap(),
        r#"{"defaultAction":"allow"}"#
    );
}

#[rstest]
#[case::off(false, None)]
#[case::on(true, Some("true"))]
fn create_body_writes_secure_access_only_when_requested(
    #[case] secure_access: bool,
    #[case] expected: Option<&str>,
) {
    let body = create_body(&CreateSandbox {
        secure_access,
        ..request()
    });
    assert_eq!(
        body.secure_access.map(|flag| flag.to_string()),
        expected.map(str::to_owned)
    );
}

#[test]
fn create_body_keeps_an_explicit_empty_mapping_and_entrypoint() {
    let empty = BTreeMap::new();
    let no_entrypoint: Vec<String> = Vec::new();
    let body = create_body(&CreateSandbox {
        metadata: Some(&empty),
        env_vars: Some(&empty),
        resource_limits: Some(&empty),
        resource_requests: Some(&empty),
        entrypoint: Some(&no_entrypoint),
        ..request()
    });
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"image":{"uri":"opensandbox/code-interpreter:v1.1.0"},"entrypoint":[],"timeout":300,"resourceLimits":{},"metadata":{},"env":{},"resourceRequests":{},"networkPolicy":{"defaultAction":"deny","egress":[]}}"#
    );
}

#[test]
fn default_resource_limits_are_one_cpu_and_two_gib() {
    assert_eq!(
        default_resource_limits(),
        BTreeMap::from([
            ("cpu".to_owned(), "1".to_owned()),
            ("memory".to_owned(), "2Gi".to_owned()),
        ])
    );
}
