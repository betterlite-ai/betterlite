#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::redis_cache::RedisCache;
use tracing::Level;

use super::support::{AttachCall, RecordingCaches, capture_logs, json, messages, standalone_cache};
use crate::coord_redis_boot::attach::{
    CLI_SSO_SESSION_TTL_SECONDS, UsageAttach, attach_usage_redis, enable_redis_auth_cache,
};

fn attach_with<'a>(caches: &'a RecordingCaches, ttl: f64) -> UsageAttach<'a> {
    UsageAttach {
        caches: caches.usage(),
        default_redis_ttl: Some(ttl),
    }
}

fn all_caches(caches: &RecordingCaches) -> Vec<Arc<RedisCache>> {
    let mut held = caches.spend.caches();
    held.extend(caches.cli.caches());
    held.extend(caches.auth.caches());
    held
}

#[tokio::test]
async fn attach_usage_redis_backs_spend_cli_sso_and_config() {
    let redis = standalone_cache().await;
    let caches = RecordingCaches::new();
    attach_usage_redis(&redis, false, &attach_with(&caches, 120.0));
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
    assert!(caches.auth.calls().is_empty());
}

#[tokio::test]
async fn attach_usage_redis_backs_the_auth_cache_when_asked() {
    let redis = standalone_cache().await;
    let caches = RecordingCaches::new();
    attach_usage_redis(&redis, true, &attach_with(&caches, 90.0));
    assert_eq!(
        caches.auth.calls(),
        vec![AttachCall::Attached {
            default_ttl: Some(90.0)
        }]
    );
    assert_eq!(caches.calls().len(), 4);
}

#[tokio::test]
async fn attach_usage_redis_gives_every_target_the_same_client() {
    let redis = standalone_cache().await;
    let caches = RecordingCaches::new();
    attach_usage_redis(&redis, true, &attach_with(&caches, 90.0));
    let held = all_caches(&caches);
    assert_eq!(held.len(), 3);
    assert!(held.iter().all(|cache| Arc::ptr_eq(cache, &redis)));
}

#[tokio::test]
async fn attach_usage_redis_logs_the_enabled_line() {
    let redis = standalone_cache().await;
    let caches = RecordingCaches::new();
    let ((), lines) = capture_logs(|| {
        attach_usage_redis(&redis, true, &attach_with(&caches, 120.0));
    });
    let infos = messages(&lines, Level::Info);
    assert_eq!(
        infos,
        vec![
            "enable_redis_auth_cache=True: attached Redis to user_api_key_cache — virtual-key lookups are now shared across all proxy workers.".to_owned()
        ]
    );
}

#[tokio::test]
async fn attach_usage_redis_logs_the_disabled_line() {
    let redis = standalone_cache().await;
    let caches = RecordingCaches::new();
    let ((), lines) = capture_logs(|| {
        attach_usage_redis(&redis, false, &attach_with(&caches, 120.0));
    });
    let infos = messages(&lines, Level::Info);
    assert_eq!(
        infos,
        vec![
            "enable_redis_auth_cache is not set: user_api_key_cache remains in-memory only (per-worker). Set litellm_settings.enable_redis_auth_cache: true to share the auth cache across workers and reduce DB load.".to_owned()
        ]
    );
}

#[test]
fn enable_redis_auth_cache_reads_the_boolean_true() {
    let settings = json(r#"{"enable_redis_auth_cache": true}"#);
    assert!(enable_redis_auth_cache(&settings));
}

#[test]
fn enable_redis_auth_cache_ignores_the_text_true() {
    let settings = json(r#"{"enable_redis_auth_cache": "true"}"#);
    assert!(!enable_redis_auth_cache(&settings));
}

#[test]
fn enable_redis_auth_cache_ignores_a_number() {
    let settings = json(r#"{"enable_redis_auth_cache": 1}"#);
    assert!(!enable_redis_auth_cache(&settings));
}

#[test]
fn enable_redis_auth_cache_ignores_null_and_absence() {
    let null = json(r#"{"enable_redis_auth_cache": null}"#);
    assert!(!enable_redis_auth_cache(&null));
    let absent = json(r#"{"other_setting": true}"#);
    assert!(!enable_redis_auth_cache(&absent));
}
