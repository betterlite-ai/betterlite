#![cfg(test)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::SecretValue;
use betterlite_secret_managers::secret_lookup::ports::{
    OidcHttp, OidcHttpError, OidcRequest, OidcResponse, OidcTokenCache,
};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::{JoinSet, yield_now};

use super::support::{FakeSecrets, NoopOidc, block, environment, secret_lookup};
use crate::coord_redis_boot::env_refs::{ResolveContext, resolve_params};
use crate::coord_redis_boot::error::CoordRedisError;
use crate::proxy_request_contracts::coordination_redis::BoolOrText;

#[tokio::test]
async fn resolve_params_replaces_a_prefixed_value_from_the_environment() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_HOST", "redis.internal")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let params = resolve_params(&block(r#"{"host": "os.environ/COORD_HOST"}"#), &context)
        .await
        .expect("the reference resolves");
    assert_eq!(params.host.as_deref(), Some("redis.internal"));
}

#[tokio::test]
async fn resolve_params_leaves_a_missing_reference_unset() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let params = resolve_params(&block(r#"{"host": "os.environ/ABSENT"}"#), &context)
        .await
        .expect("an unresolved reference is unset");
    assert!(params.host.is_none());
}

#[tokio::test]
async fn resolve_params_coerces_the_port_reference_to_a_number() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_PORT", "6380")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let params = resolve_params(&block(r#"{"port": "os.environ/COORD_PORT"}"#), &context)
        .await
        .expect("the port reference resolves");
    assert_eq!(params.port, Some(6380));
}

#[tokio::test]
async fn resolve_params_turns_an_empty_port_reference_into_none() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_PORT", "")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let params = resolve_params(&block(r#"{"port": "os.environ/COORD_PORT"}"#), &context)
        .await
        .expect("an empty reference counts as unset");
    assert!(params.port.is_none());
}

#[tokio::test]
async fn resolve_params_coerces_the_ssl_reference_to_a_bool() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_SSL", "TRUE"), ("COORD_IAM", "off")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let text = r#"{"ssl": "os.environ/COORD_SSL", "aws_iam_serverless": "os.environ/COORD_IAM"}"#;
    let params = resolve_params(&block(text), &context)
        .await
        .expect("the flag references resolve");
    assert_eq!(params.ssl, Some(true));
    assert_eq!(params.aws_iam_serverless, Some(BoolOrText::Bool(false)));
}

#[tokio::test]
async fn resolve_params_rejects_a_boolean_reference_outside_a_typed_field() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_USER", "true")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let error = resolve_params(&block(r#"{"username": "os.environ/COORD_USER"}"#), &context)
        .await
        .expect_err("a username is text, not a boolean");
    assert!(matches!(error, CoordRedisError::Params(_)));
}

#[tokio::test]
async fn resolve_params_reads_through_the_secret_lookup() {
    let secrets = Arc::new(secret_lookup());
    let env = environment(&[
        ("COORD_HOST", "coordinator.internal"),
        ("COORD_SSL", "true"),
    ]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let text = r#"{"host": "os.environ/COORD_HOST", "ssl": "os.environ/COORD_SSL"}"#;
    let params = resolve_params(&block(text), &context)
        .await
        .expect("the environment references resolve");
    assert_eq!(params.host.as_deref(), Some("coordinator.internal"));
    assert_eq!(params.ssl, Some(true));
}

#[tokio::test]
async fn resolve_params_uses_a_looked_up_password_over_the_environment() {
    let secrets = FakeSecrets::new(&[(
        "COORD_PASSWORD",
        SecretValue::Text(SecretString::from("from-the-manager")),
    )]);
    let env = environment(&[("COORD_PASSWORD", "from-the-env")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let params = resolve_params(
        &block(r#"{"password": "os.environ/COORD_PASSWORD"}"#),
        &context,
    )
    .await
    .expect("the password reference resolves");
    let password = params.password.as_ref().expect("the password is set");
    assert_eq!(password.expose_secret(), "from-the-manager");
}

#[tokio::test]
async fn resolve_params_keeps_the_url_reference_in_a_secret_string() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_URL", "redis://:s3cr3t@coordinator.internal:6379")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let params = resolve_params(&block(r#"{"url": "os.environ/COORD_URL"}"#), &context)
        .await
        .expect("the url reference resolves");
    let url = params.url.as_ref().expect("the url is set");
    assert_eq!(
        url.expose_secret(),
        "redis://:s3cr3t@coordinator.internal:6379"
    );
}

#[tokio::test]
async fn resolve_params_keeps_a_password_literal_in_a_secret_string() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let params = resolve_params(&block(r#"{"password": "literal-secret"}"#), &context)
        .await
        .expect("a literal password needs no lookup");
    let password = params.password.as_ref().expect("the password is set");
    assert_eq!(password.expose_secret(), "literal-secret");
}

#[tokio::test]
async fn resolve_params_rejects_a_boolean_secret_in_the_password_field() {
    let secrets = FakeSecrets::new(&[("COORD_PASSWORD", SecretValue::Bool(true))]);
    let env = environment(&[]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let error = resolve_params(
        &block(r#"{"password": "os.environ/COORD_PASSWORD"}"#),
        &context,
    )
    .await
    .expect_err("a password must be text");
    assert!(matches!(error, CoordRedisError::Params(_)));
    assert!(error.to_string().contains("password"), "{error}");
}

#[tokio::test]
async fn resolve_params_leaves_values_without_the_prefix_alone() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_HOST", "redis.internal")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let text = r#"{"host": "redis.internal", "port": 6379, "ssl": true, "password": null, "service_name": "environment/COORD_HOST", "startup_nodes": [{"host": "os.environ/COORD_HOST", "port": 6379}]}"#;
    let params = resolve_params(&block(text), &context)
        .await
        .expect("nothing needs resolving");
    assert_eq!(params.host.as_deref(), Some("redis.internal"));
    assert_eq!(params.port, Some(6379));
    assert_eq!(params.ssl, Some(true));
    assert!(params.password.is_none());
    assert_eq!(
        params.service_name.as_deref(),
        Some("environment/COORD_HOST")
    );
    let nodes = params.startup_nodes.expect("the nodes parse");
    assert_eq!(nodes[0].host, "os.environ/COORD_HOST");
}

#[tokio::test]
async fn resolve_params_rejects_a_block_that_is_not_a_mapping() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let error = resolve_params(&block(r#""redis://localhost""#), &context)
        .await
        .expect_err("a non-mapping block fails");
    assert!(matches!(error, CoordRedisError::NotMapping));
    assert_eq!(
        error.to_string(),
        "general_settings.coordination_redis must be a mapping of Redis connection params"
    );
    let array_error = resolve_params(&block("[1, 2]"), &context)
        .await
        .expect_err("an array block fails");
    assert!(matches!(array_error, CoordRedisError::NotMapping));
}

#[tokio::test]
async fn resolve_params_names_a_reference_with_the_wrong_typed_value() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_PORT", "not-a-number")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let error = resolve_params(&block(r#"{"port": "os.environ/COORD_PORT"}"#), &context)
        .await
        .expect_err("the port text does not parse");
    assert!(matches!(error, CoordRedisError::Environment(_)));
    assert_eq!(
        error.to_string(),
        "environment variable COORD_PORT must be a number"
    );
}

#[tokio::test]
async fn resolve_params_names_a_flag_reference_that_is_not_a_boolean_word() {
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("COORD_SSL", "maybe")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let error = resolve_params(&block(r#"{"ssl": "os.environ/COORD_SSL"}"#), &context)
        .await
        .expect_err("the ssl text is not a boolean word");
    assert_eq!(
        error.to_string(),
        "environment variable COORD_SSL must be a bool"
    );
}

struct BlockingOidc {
    release: Mutex<Receiver<()>>,
    blocked_the_worker: Arc<AtomicBool>,
}

impl OidcHttp for BlockingOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        let release = self.release.lock().unwrap_or_else(PoisonError::into_inner);
        if release.recv_timeout(BLOCKING_TIMEOUT).is_err() {
            self.blocked_the_worker.store(true, Ordering::SeqCst);
            return Err(OidcHttpError::Request {
                message: "the OIDC read blocked the async worker".to_owned(),
            });
        }
        Ok(OidcResponse {
            status: 200,
            body: b"offloaded-token".to_vec(),
        })
    }
}

const BLOCKING_TIMEOUT: Duration = Duration::from_secs(2);

// The async worker must stay free while a real SecretLookup does its blocking OIDC read.
#[tokio::test]
async fn secret_lookup_runs_a_blocking_oidc_read_off_the_worker() {
    let (sender, receiver) = sync_channel(0);
    let blocked_the_worker = Arc::new(AtomicBool::new(false));
    let lookup = Arc::new(SecretLookup {
        manager: None,
        oidc_http: Arc::new(BlockingOidc {
            release: Mutex::new(receiver),
            blocked_the_worker: Arc::clone(&blocked_the_worker),
        }),
        azure_tokens: Arc::new(NoopOidc),
        clock: Arc::new(NoopOidc),
        oidc_cache: OidcTokenCache::default(),
    });
    let mut signallers = JoinSet::new();
    signallers.spawn(async move {
        yield_now().await;
        sender.send(()).expect("the OIDC read still waits");
    });
    let env = no_environment();
    let context = ResolveContext {
        secrets: &lookup,
        env: &env,
    };
    let outcome = resolve_params(
        &block(r#"{"password": "os.environ/oidc/google/audience"}"#),
        &context,
    )
    .await;
    assert!(
        !blocked_the_worker.load(Ordering::SeqCst),
        "the blocking OIDC read ran on the async worker"
    );
    let params = outcome.expect("the offloaded lookup resolves the token");
    assert!(
        signallers
            .join_next()
            .await
            .expect("the signaller ran")
            .is_ok()
    );
    let password = params.password.expect("the password is set");
    assert_eq!(password.expose_secret(), "offloaded-token");
}

#[tokio::test]
async fn resolve_params_reports_a_lookup_failure() {
    let secrets = FakeSecrets::failing();
    let env = environment(&[]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let error = resolve_params(&block(r#"{"host": "os.environ/COORD_HOST"}"#), &context)
        .await
        .expect_err("the lookup fails");
    assert!(matches!(error, CoordRedisError::Secrets(_)));
    assert_eq!(error.to_string(), "Google OIDC provider failed");
}
