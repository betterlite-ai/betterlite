#![cfg(test)]

use betterlite_secret_managers::secret_lookup::SecretValue;
use secrecy::SecretString;

use super::support::{FakeSecrets, environment, json};
use crate::coord_redis_boot::env_refs::{ResolveContext, resolve_env_refs};
use crate::coord_redis_boot::error::CoordRedisError;

#[test]
fn resolve_env_refs_replaces_a_prefixed_value_from_the_environment() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_HOST", "redis.internal")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let resolved = resolve_env_refs(&json(r#"{"host": "os.environ/COORD_HOST"}"#), &context)
        .expect("the reference resolves");
    assert_eq!(resolved, json(r#"{"host": "redis.internal"}"#));
}

#[test]
fn resolve_env_refs_leaves_a_missing_reference_null() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let resolved = resolve_env_refs(&json(r#"{"host": "os.environ/ABSENT"}"#), &context)
        .expect("an unresolved reference is null");
    assert_eq!(resolved, json(r#"{"host": null}"#));
}

#[test]
fn resolve_env_refs_coerces_the_port_reference_to_a_number() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_PORT", "6380")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let resolved = resolve_env_refs(&json(r#"{"port": "os.environ/COORD_PORT"}"#), &context)
        .expect("the port reference resolves");
    assert_eq!(resolved, json(r#"{"port": 6380}"#));
}

#[test]
fn resolve_env_refs_turns_an_empty_port_reference_null() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_PORT", "")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let resolved = resolve_env_refs(&json(r#"{"port": "os.environ/COORD_PORT"}"#), &context)
        .expect("an empty reference counts as unset");
    assert_eq!(resolved, json(r#"{"port": null}"#));
}

#[test]
fn resolve_env_refs_coerces_the_ssl_reference_to_a_bool() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_SSL", "TRUE"), ("COORD_IAM", "off")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let block =
        json(r#"{"ssl": "os.environ/COORD_SSL", "aws_iam_serverless": "os.environ/COORD_IAM"}"#);
    let resolved = resolve_env_refs(&block, &context).expect("the flag references resolve");
    assert_eq!(
        resolved,
        json(r#"{"ssl": true, "aws_iam_serverless": false}"#)
    );
}

#[test]
fn resolve_env_refs_keeps_a_bool_reference_outside_a_typed_field_a_bool() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_USER", "true")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let resolved = resolve_env_refs(&json(r#"{"username": "os.environ/COORD_USER"}"#), &context)
        .expect("the username reference resolves");
    assert_eq!(resolved, json(r#"{"username": true}"#));
}

#[test]
fn resolve_env_refs_uses_a_looked_up_value_over_the_environment() {
    let secrets = FakeSecrets::new(&[(
        "COORD_PASSWORD",
        SecretValue::Text(SecretString::from("from-the-manager")),
    )]);
    let env = environment(&[("COORD_PASSWORD", "from-the-env")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let resolved = resolve_env_refs(
        &json(r#"{"password": "os.environ/COORD_PASSWORD"}"#),
        &context,
    )
    .expect("the password reference resolves");
    assert_eq!(resolved, json(r#"{"password": "from-the-manager"}"#));
}

#[test]
fn resolve_env_refs_leaves_values_without_the_prefix_alone() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_HOST", "redis.internal")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let block = json(
        r#"{"host": "redis.internal", "port": 6379, "ssl": true, "password": null, "service_name": "environment/COORD_HOST", "startup_nodes": [{"host": "os.environ/COORD_HOST", "port": 6379}]}"#,
    );
    let resolved = resolve_env_refs(&block, &context).expect("nothing needs resolving");
    assert_eq!(resolved, block);
}

#[test]
fn resolve_env_refs_rejects_a_block_that_is_not_a_mapping() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let error = resolve_env_refs(&json(r#""redis://localhost""#), &context)
        .expect_err("a non-mapping block fails");
    assert!(matches!(error, CoordRedisError::NotMapping));
    assert_eq!(
        error.to_string(),
        "general_settings.coordination_redis must be a mapping of Redis connection params"
    );
    let array_error =
        resolve_env_refs(&json("[1, 2]"), &context).expect_err("an array block fails");
    assert!(matches!(array_error, CoordRedisError::NotMapping));
}

#[test]
fn resolve_env_refs_names_a_reference_with_the_wrong_typed_value() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_PORT", "not-a-number")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let error = resolve_env_refs(&json(r#"{"port": "os.environ/COORD_PORT"}"#), &context)
        .expect_err("the port text does not parse");
    assert!(matches!(error, CoordRedisError::Environment(_)));
    assert_eq!(
        error.to_string(),
        "environment variable COORD_PORT must be a number"
    );
}

#[test]
fn resolve_env_refs_names_a_flag_reference_that_is_not_a_boolean_word() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_SSL", "maybe")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let error = resolve_env_refs(&json(r#"{"ssl": "os.environ/COORD_SSL"}"#), &context)
        .expect_err("the ssl text is not a boolean word");
    assert_eq!(
        error.to_string(),
        "environment variable COORD_SSL must be a bool"
    );
}

#[test]
fn resolve_env_refs_reports_a_lookup_failure() {
    let secrets = FakeSecrets::failing();
    let env = environment(&[]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let error = resolve_env_refs(&json(r#"{"host": "os.environ/COORD_HOST"}"#), &context)
        .expect_err("the lookup fails");
    assert!(matches!(error, CoordRedisError::Secrets(_)));
    assert_eq!(error.to_string(), "Google OIDC provider failed");
}
