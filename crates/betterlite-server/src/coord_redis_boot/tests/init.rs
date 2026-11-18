#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use tracing::Level;

use super::support::{
    AttachCall, FakeSecrets, LogLine, PingOutcome, RecordingCaches, RecordingProbe,
    capture_async_logs, closed_port, environment, json, messages, no_environment, standalone_cache,
    wiring,
};
use crate::coord_redis_boot::attach::{CLI_SSO_SESSION_TTL_SECONDS, UsageAttach};
use crate::coord_redis_boot::env_refs::ResolveContext;
use crate::coord_redis_boot::error::CoordRedisError;
use crate::coord_redis_boot::init::{
    buffer_redis, init_env_fallback, init_from_config, init_from_db,
};

const CONFIG_INFO: &str = "coordination_redis: using a standalone Redis from general_settings for usage tracking, rate limiting, and cross-pod coordination.";
const DB_INFO: &str = "coordination_redis: using the standalone Redis saved in the database for usage tracking, rate limiting, and cross-pod coordination.";
const ENV_INFO: &str = "coordination_redis: using a standalone Redis built from REDIS_* environment variables for usage tracking, rate limiting, and cross-pod coordination.";
const DB_NO_TARGET: &str =
    "coordination_redis saved in the database names no connection target; ignoring it.";
const BUILD_FAILED: &str =
    "coordination_redis: could not build a Redis client from REDIS_* environment variables (";
const NOT_REACHABLE: &str =
    "coordination_redis: REDIS_* environment variables named a Redis that is not reachable (";
const STAYS_IN_MEMORY: &str = "); cross-pod coordination stays in-memory. Set general_settings.coordination_redis explicitly to require it.";

fn config_with_block(block: &str) -> JsonValue {
    json(&format!(
        r#"{{"general_settings": {{"coordination_redis": {block}}}}}"#
    ))
}

fn only_warning(lines: &[LogLine]) -> String {
    let warnings = messages(lines, Level::Warn);
    assert_eq!(warnings.len(), 1, "exactly one warning is logged");
    warnings.first().cloned().unwrap_or_default()
}

fn assert_info(lines: &[LogLine], expected: &str) {
    let infos = messages(lines, Level::Info);
    assert_eq!(
        infos.len(),
        2,
        "the attachment line and the source line are logged"
    );
    assert_eq!(infos.last().map(String::as_str), Some(expected));
}

fn attach_of<'a>(caches: &'a RecordingCaches) -> UsageAttach<'a> {
    UsageAttach {
        caches: caches.usage(),
        default_redis_ttl: Some(120.0),
    }
}

fn assert_backs_spend_cli_sso_and_config(caches: &RecordingCaches) {
    assert_eq!(
        caches.spend.calls(),
        vec![AttachCall::Attached {
            default_ttl: Some(120.0)
        }]
    );
    assert_eq!(
        caches.cli.calls(),
        vec![AttachCall::Attached {
            default_ttl: Some(CLI_SSO_SESSION_TTL_SECONDS)
        }]
    );
    assert_eq!(caches.calls().last(), Some(&AttachCall::Replaced));
}

fn port_environment(port: u16) -> BTreeMap<String, String> {
    environment(&[
        ("REDIS_HOST", "127.0.0.1"),
        ("REDIS_PORT", &port.to_string()),
        ("REDIS_SOCKET_TIMEOUT", "1"),
        ("REDIS_SOCKET_CONNECT_TIMEOUT", "1"),
    ])
}

async fn config_without_a_block(config: &JsonValue) -> bool {
    let secrets = FakeSecrets::new(&[]);
    let env = no_environment();
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let caches = RecordingCaches::new();
    let cache = init_from_config(config, &context, &wiring(), &attach_of(&caches))
        .await
        .expect("an absent block is not an error");
    assert!(caches.calls().is_empty());
    cache.is_none()
}

#[tokio::test]
async fn init_from_config_returns_none_without_a_block() {
    assert!(config_without_a_block(&json("{}")).await);
    assert!(config_without_a_block(&json(r#"{"general_settings": null}"#)).await);
    assert!(config_without_a_block(&json(r#"{"general_settings": {}}"#)).await);
    assert!(
        config_without_a_block(&json(
            r#"{"general_settings": {"coordination_redis": null}}"#
        ))
        .await
    );
}

#[tokio::test]
async fn init_from_config_rejects_a_block_that_is_not_a_mapping() {
    let secrets = FakeSecrets::new(&[]);
    let env = no_environment();
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let config = config_with_block(r#""redis://localhost:6379""#);
    let caches = RecordingCaches::new();
    let error = init_from_config(&config, &context, &wiring(), &attach_of(&caches))
        .await
        .err()
        .expect("a non-mapping block stops startup");
    assert!(matches!(error, CoordRedisError::NotMapping));
    assert_eq!(
        error.to_string(),
        "general_settings.coordination_redis must be a mapping of Redis connection params"
    );
    assert!(caches.calls().is_empty());
}

#[tokio::test]
async fn init_from_config_rejects_a_block_without_a_target() {
    let secrets = FakeSecrets::new(&[]);
    let env = no_environment();
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let config = config_with_block(r#"{"port": 6379, "ssl": true}"#);
    let caches = RecordingCaches::new();
    let error = init_from_config(&config, &context, &wiring(), &attach_of(&caches))
        .await
        .err()
        .expect("a block without a target stops startup");
    assert!(matches!(error, CoordRedisError::MissingTarget));
    assert_eq!(
        error.to_string(),
        "general_settings.coordination_redis needs a connection target: set one of host, url, startup_nodes, or sentinel_nodes"
    );
    assert!(caches.calls().is_empty());
    let empty = config_with_block("{}");
    let empty_error = init_from_config(&empty, &context, &wiring(), &attach_of(&caches))
        .await
        .err()
        .expect("an empty block names no target");
    assert!(matches!(empty_error, CoordRedisError::MissingTarget));
    assert!(caches.calls().is_empty());
}

#[tokio::test]
async fn init_from_config_builds_and_attaches() {
    let port = closed_port();
    let config = config_with_block(&format!(r#"{{"host": "127.0.0.1", "port": {port}}}"#));
    let secrets = FakeSecrets::new(&[]);
    let env = no_environment();
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let caches = RecordingCaches::new();
    let (result, lines) = capture_async_logs(init_from_config(
        &config,
        &context,
        &wiring(),
        &attach_of(&caches),
    ))
    .await;
    let cache = result
        .expect("the block builds")
        .expect("the block names a target");
    assert_eq!(cache.transport.family, RedisCacheFamily::Standalone);
    assert_eq!(cache.transport.settings.port, Some(i64::from(port)));
    assert_info(&lines, CONFIG_INFO);
    assert_backs_spend_cli_sso_and_config(&caches);
    assert!(caches.auth.calls().is_empty());
}

#[tokio::test]
async fn init_from_config_resolves_environment_references() {
    let port = closed_port();
    let config =
        config_with_block(r#"{"host": "os.environ/COORD_HOST", "port": "os.environ/COORD_PORT"}"#);
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[
        ("COORD_HOST", "127.0.0.1"),
        ("COORD_PORT", &port.to_string()),
    ]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let caches = RecordingCaches::new();
    let cache = init_from_config(&config, &context, &wiring(), &attach_of(&caches))
        .await
        .expect("the references resolve")
        .expect("the block names a target");
    assert_eq!(cache.transport.settings.host.as_deref(), Some("127.0.0.1"));
    assert_eq!(cache.transport.settings.port, Some(i64::from(port)));
}

#[tokio::test]
async fn init_from_config_reads_the_auth_flag_from_litellm_settings() {
    let port = closed_port();
    let block = format!(r#"{{"host": "127.0.0.1", "port": {port}}}"#);
    let config = json(&format!(
        r#"{{"general_settings": {{"coordination_redis": {block}}}, "litellm_settings": {{"enable_redis_auth_cache": true}}}}"#
    ));
    let text_config = json(&format!(
        r#"{{"general_settings": {{"coordination_redis": {block}}}, "litellm_settings": {{"enable_redis_auth_cache": "true"}}}}"#
    ));
    let secrets = FakeSecrets::new(&[]);
    let env = no_environment();
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let enabled = RecordingCaches::new();
    init_from_config(&config, &context, &wiring(), &attach_of(&enabled))
        .await
        .expect("the block builds");
    let disabled = RecordingCaches::new();
    init_from_config(&text_config, &context, &wiring(), &attach_of(&disabled))
        .await
        .expect("the text flag leaves the auth cache alone");
    assert_eq!(enabled.auth.calls().len(), 1);
    assert!(disabled.auth.calls().is_empty());
}

#[tokio::test]
async fn init_from_config_reports_a_malformed_node_list() {
    let port = closed_port();
    let config = config_with_block(&format!(r#"{{"host": "127.0.0.1", "port": {port}}}"#));
    let secrets = FakeSecrets::new(&[]);
    let env = environment(&[("REDIS_CLUSTER_NODES", "not json")]);
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let caches = RecordingCaches::new();
    let error = init_from_config(&config, &context, &wiring(), &attach_of(&caches))
        .await
        .err()
        .expect("a malformed node list stops an explicit block");
    assert!(matches!(error, CoordRedisError::ClusterNodes { .. }));
}

#[tokio::test]
async fn init_from_config_reports_invalid_params() {
    let config = config_with_block(r#"{"host": 5}"#);
    let secrets = FakeSecrets::new(&[]);
    let env = no_environment();
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let caches = RecordingCaches::new();
    let error = init_from_config(&config, &context, &wiring(), &attach_of(&caches))
        .await
        .err()
        .expect("a host that is not text stops startup");
    assert!(matches!(error, CoordRedisError::Params(_)));
}

#[tokio::test]
async fn init_from_db_returns_none_without_a_block() {
    let secrets = FakeSecrets::new(&[]);
    let env = no_environment();
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let caches = RecordingCaches::new();
    let (result, lines) = capture_async_logs(init_from_db(
        None,
        &json("{}"),
        &context,
        &wiring(),
        &attach_of(&caches),
    ))
    .await;
    assert!(result.expect("nothing persisted is not an error").is_none());
    assert!(lines.is_empty());
    assert!(caches.calls().is_empty());
}

#[tokio::test]
async fn init_from_db_warns_without_a_target() {
    let persisted = json(r#"{"port": 6379}"#);
    let secrets = FakeSecrets::new(&[]);
    let env = no_environment();
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let caches = RecordingCaches::new();
    let (result, lines) = capture_async_logs(init_from_db(
        Some(&persisted),
        &json("{}"),
        &context,
        &wiring(),
        &attach_of(&caches),
    ))
    .await;
    assert!(result.expect("a targetless block is ignored").is_none());
    assert_eq!(only_warning(&lines), DB_NO_TARGET);
    assert!(caches.calls().is_empty());
}

#[tokio::test]
async fn init_from_db_builds_and_attaches() {
    let port = closed_port();
    let persisted = json(&format!(r#"{{"host": "127.0.0.1", "port": {port}}}"#));
    let settings = json(r#"{"enable_redis_auth_cache": true}"#);
    let secrets = FakeSecrets::new(&[]);
    let env = no_environment();
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let caches = RecordingCaches::new();
    let (result, lines) = capture_async_logs(init_from_db(
        Some(&persisted),
        &settings,
        &context,
        &wiring(),
        &attach_of(&caches),
    ))
    .await;
    let cache = result
        .expect("the saved block builds")
        .expect("the block names a target");
    assert_eq!(cache.transport.settings.port, Some(i64::from(port)));
    assert_info(&lines, DB_INFO);
    assert_backs_spend_cli_sso_and_config(&caches);
    assert_eq!(caches.auth.calls().len(), 1);
}

#[tokio::test]
async fn init_from_db_rejects_a_block_that_is_not_a_mapping() {
    let persisted = json(r#"["host", 6379]"#);
    let secrets = FakeSecrets::new(&[]);
    let env = no_environment();
    let context = ResolveContext {
        secrets: &secrets,
        env: &env,
    };
    let caches = RecordingCaches::new();
    let error = init_from_db(
        Some(&persisted),
        &json("{}"),
        &context,
        &wiring(),
        &attach_of(&caches),
    )
    .await
    .err()
    .expect("a saved list is not a mapping");
    assert!(matches!(error, CoordRedisError::NotMapping));
}

#[tokio::test]
async fn init_env_fallback_returns_none_without_a_target() {
    let caches = RecordingCaches::new();
    let probe = RecordingProbe {
        outcome: PingOutcome::Reachable,
    };
    let (result, lines) = capture_async_logs(init_env_fallback(
        &json("{}"),
        &wiring(),
        &no_environment(),
        &probe,
        &attach_of(&caches),
    ))
    .await;
    assert!(result.is_none());
    assert!(lines.is_empty());
    assert!(caches.calls().is_empty());
}

#[tokio::test]
async fn init_env_fallback_warns_when_the_build_fails() {
    let env = environment(&[("REDIS_CLUSTER_NODES", "not json")]);
    let caches = RecordingCaches::new();
    let probe = RecordingProbe {
        outcome: PingOutcome::Reachable,
    };
    let (result, lines) = capture_async_logs(init_env_fallback(
        &json("{}"),
        &wiring(),
        &env,
        &probe,
        &attach_of(&caches),
    ))
    .await;
    assert!(result.is_none());
    let warning = only_warning(&lines);
    assert!(warning.starts_with(BUILD_FAILED), "{warning}");
    assert!(warning.ends_with(STAYS_IN_MEMORY), "{warning}");
    assert!(
        warning.contains("REDIS_CLUSTER_NODES is not valid JSON"),
        "{warning}"
    );
    assert!(caches.calls().is_empty());
}

#[tokio::test]
async fn init_env_fallback_warns_when_the_ping_fails() {
    let env = port_environment(closed_port());
    let caches = RecordingCaches::new();
    let probe = RecordingProbe {
        outcome: PingOutcome::Fails,
    };
    let (result, lines) = capture_async_logs(init_env_fallback(
        &json("{}"),
        &wiring(),
        &env,
        &probe,
        &attach_of(&caches),
    ))
    .await;
    assert!(result.is_none());
    let warning = only_warning(&lines);
    assert!(warning.starts_with(NOT_REACHABLE), "{warning}");
    assert!(
        warning.contains("Redis circuit breaker is open, skipping ping"),
        "{warning}"
    );
    assert!(warning.ends_with(STAYS_IN_MEMORY), "{warning}");
    assert!(caches.calls().is_empty());
}

#[tokio::test(start_paused = true)]
async fn init_env_fallback_warns_when_the_ping_times_out() {
    let env = port_environment(closed_port());
    let caches = RecordingCaches::new();
    let probe = RecordingProbe {
        outcome: PingOutcome::Hangs,
    };
    let (result, lines) = capture_async_logs(init_env_fallback(
        &json("{}"),
        &wiring(),
        &env,
        &probe,
        &attach_of(&caches),
    ))
    .await;
    assert!(result.is_none());
    let warning = only_warning(&lines);
    assert!(warning.starts_with(NOT_REACHABLE), "{warning}");
    assert!(warning.contains("deadline has elapsed"), "{warning}");
    assert!(warning.ends_with(STAYS_IN_MEMORY), "{warning}");
    assert!(caches.calls().is_empty());
}

#[tokio::test]
async fn init_env_fallback_stays_silent_on_a_false_ping() {
    let env = port_environment(closed_port());
    let caches = RecordingCaches::new();
    let probe = RecordingProbe {
        outcome: PingOutcome::Unreachable,
    };
    let (result, lines) = capture_async_logs(init_env_fallback(
        &json("{}"),
        &wiring(),
        &env,
        &probe,
        &attach_of(&caches),
    ))
    .await;
    assert!(result.is_none());
    assert!(messages(&lines, Level::Warn).is_empty());
    assert!(messages(&lines, Level::Info).is_empty());
    assert!(caches.calls().is_empty());
}

#[tokio::test]
async fn init_env_fallback_attaches_when_reachable() {
    let port = closed_port();
    let env = port_environment(port);
    let caches = RecordingCaches::new();
    let probe = RecordingProbe {
        outcome: PingOutcome::Reachable,
    };
    let (result, lines) = capture_async_logs(init_env_fallback(
        &json(r#"{"enable_redis_auth_cache": true}"#),
        &wiring(),
        &env,
        &probe,
        &attach_of(&caches),
    ))
    .await;
    let cache = result.expect("a reachable environment Redis is used");
    assert_eq!(cache.transport.family, RedisCacheFamily::Standalone);
    assert_eq!(cache.transport.settings.port, Some(i64::from(port)));
    assert_info(&lines, ENV_INFO);
    assert_backs_spend_cli_sso_and_config(&caches);
    assert_eq!(caches.auth.calls().len(), 1);
}

#[tokio::test]
async fn buffer_redis_returns_the_usage_cache_first() {
    let usage = standalone_cache().await;
    let env = environment(&[("REDIS_HOST", "127.0.0.1"), ("REDIS_PORT", "6379")]);
    let settings = json(r#"{"use_redis_transaction_buffer": true}"#);
    let redis = buffer_redis(&settings, Some(&usage), &wiring(), &env)
        .await
        .expect("the usage cache wins");
    assert!(Arc::ptr_eq(&redis.expect("a buffer client"), &usage));
}

#[tokio::test]
async fn buffer_redis_builds_from_the_environment_when_wanted() {
    let port = closed_port();
    let env = port_environment(port);
    let bool_flag = json(r#"{"use_redis_transaction_buffer": true}"#);
    let redis = buffer_redis(&bool_flag, None, &wiring(), &env)
        .await
        .expect("the flag builds a client")
        .expect("the environment names a target");
    assert_eq!(redis.transport.settings.port, Some(i64::from(port)));
    let text_flag = json(r#"{"use_redis_transaction_buffer": "TRUE"}"#);
    let from_text = buffer_redis(&text_flag, None, &wiring(), &env)
        .await
        .expect("a true text builds a client");
    assert!(from_text.is_some());
}

#[tokio::test]
async fn buffer_redis_returns_none_when_the_flag_is_off() {
    let env = environment(&[("REDIS_HOST", "127.0.0.1"), ("REDIS_PORT", "6379")]);
    let off = json(r#"{"use_redis_transaction_buffer": false}"#);
    let bool_off = buffer_redis(&off, None, &wiring(), &env)
        .await
        .expect("a disabled buffer is not an error");
    assert!(bool_off.is_none());
    let text_off = json(r#"{"use_redis_transaction_buffer": "false"}"#);
    let from_text = buffer_redis(&text_off, None, &wiring(), &env)
        .await
        .expect("a false text is not an error");
    assert!(from_text.is_none());
    let unrecognized = json(r#"{"use_redis_transaction_buffer": "no"}"#);
    let from_word = buffer_redis(&unrecognized, None, &wiring(), &env)
        .await
        .expect("an unknown word is off");
    assert!(from_word.is_none());
    let null = json(r#"{"use_redis_transaction_buffer": null}"#);
    let from_null = buffer_redis(&null, None, &wiring(), &env)
        .await
        .expect("null takes the default");
    assert!(from_null.is_none());
    let absent = buffer_redis(&json("{}"), None, &wiring(), &env)
        .await
        .expect("an absent flag takes the default");
    assert!(absent.is_none());
}

#[tokio::test]
async fn buffer_redis_without_a_target_returns_none() {
    let settings = json(r#"{"use_redis_transaction_buffer": true}"#);
    let redis = buffer_redis(&settings, None, &wiring(), &no_environment())
        .await
        .expect("no target is not an error");
    assert!(redis.is_none());
}

#[tokio::test]
async fn buffer_redis_reports_a_build_error() {
    let settings = json(r#"{"use_redis_transaction_buffer": true}"#);
    let env = environment(&[("REDIS_CLUSTER_NODES", "not json")]);
    let error = buffer_redis(&settings, None, &wiring(), &env)
        .await
        .err()
        .expect("the node list does not parse");
    assert!(matches!(error, CoordRedisError::ClusterNodes { .. }));
}
