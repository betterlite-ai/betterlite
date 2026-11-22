use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;

use super::support::{environment, object};
use crate::coord_redis_boot::env_refs::{EnvironmentSecrets, ResolveContext};
use crate::coordination_redis_settings::validate::{ValidationError, validated_params};

struct Fixture {
    secrets: EnvironmentSecrets,
    environment: BTreeMap<String, String>,
}

impl Fixture {
    fn new(pairs: &[(&str, &str)]) -> Self {
        Self {
            secrets: EnvironmentSecrets,
            environment: environment(pairs),
        }
    }

    fn context(&self) -> ResolveContext<'_> {
        ResolveContext {
            secrets: &self.secrets,
            env: &self.environment,
        }
    }
}

#[tokio::test]
async fn names_each_rejected_field_once_and_sorted() {
    let fixture = Fixture::new(&[]);
    let settings = object(
        r#"{"host": "h", "port": "not-a-port", "ssl": "maybe", "startup_nodes": [{"host": "a"}, {"host": "b"}]}"#,
    );
    let error = validated_params(&settings, &fixture.context())
        .await
        .expect_err("invalid fields are reported");
    let ValidationError::InvalidFields(invalid) = error else {
        panic!("expected the invalid field list");
    };
    assert_eq!(invalid.0, "['port', 'ssl', 'startup_nodes']");
    assert_eq!(
        invalid.to_string(),
        "Invalid coordination_redis settings for fields: ['port', 'ssl', 'startup_nodes']",
    );
}

#[tokio::test]
async fn requires_a_connection_target() {
    let fixture = Fixture::new(&[]);
    let settings = object(r#"{"port": 6379, "username": "u"}"#);
    let error = validated_params(&settings, &fixture.context())
        .await
        .expect_err("a port alone is no target");
    assert!(matches!(error, ValidationError::MissingTarget));
}

#[tokio::test]
async fn an_empty_host_or_node_list_still_selects_a_target() {
    let fixture = Fixture::new(&[]);
    let params = validated_params(&object(r#"{"host": ""}"#), &fixture.context())
        .await
        .unwrap();
    assert_eq!(params.host.as_deref(), Some(""));
    let node_params = validated_params(&object(r#"{"startup_nodes": []}"#), &fixture.context())
        .await
        .unwrap();
    assert_eq!(node_params.startup_nodes, Some(Vec::new()));
}

#[tokio::test]
async fn resolves_environment_references_without_touching_the_settings() {
    let fixture = Fixture::new(&[("REDIS_HOST", "cache.internal"), ("REDIS_PORT", "6380")]);
    let settings = object(
        r#"{"host": "os.environ/REDIS_HOST", "port": "os.environ/REDIS_PORT", "password": "os.environ/REDIS_PASSWORD"}"#,
    );
    let params = validated_params(&settings, &fixture.context())
        .await
        .unwrap();
    assert_eq!(params.host.as_deref(), Some("cache.internal"));
    assert_eq!(params.port, Some(6380));
    assert!(params.password.is_none());
    assert_eq!(
        settings.get("host").and_then(JsonValue::as_str),
        Some("os.environ/REDIS_HOST"),
    );
}

#[tokio::test]
async fn an_unset_reference_leaves_no_connection_target() {
    let fixture = Fixture::new(&[]);
    let settings = object(r#"{"host": "os.environ/MISSING"}"#);
    let error = validated_params(&settings, &fixture.context())
        .await
        .expect_err("an unset reference names no target");
    assert!(matches!(error, ValidationError::MissingTarget));
}

#[tokio::test]
async fn a_malformed_environment_value_names_its_field() {
    let fixture = Fixture::new(&[("REDIS_PORT", "not-a-number")]);
    let settings = object(r#"{"host": "h", "port": "os.environ/REDIS_PORT"}"#);
    let error = validated_params(&settings, &fixture.context())
        .await
        .expect_err("a malformed port is rejected");
    let ValidationError::InvalidFields(invalid) = error else {
        panic!("expected the invalid field list");
    };
    assert_eq!(invalid.0, "['port']");
}

#[tokio::test]
async fn a_non_object_settings_value_is_a_resolution_error() {
    let fixture = Fixture::new(&[]);
    let settings = JsonValue::Array(Vec::new());
    let error = validated_params(&settings, &fixture.context())
        .await
        .expect_err("a list is no settings block");
    assert!(matches!(error, ValidationError::Resolution(_)));
}
