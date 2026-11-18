#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_redis::redis_cache::RedisCache;
use tracing::Level;

use super::support::{
    AttachCall, RecordingCaches, auth_cache, capture_logs, dual_cache, messages, standalone_cache,
};
use crate::coord_redis_boot::attach::{
    CLI_SSO_SESSION_TTL_SECONDS, ConfigCacheTarget, UsageAttach, UsageCacheTarget,
    attach_usage_redis,
};

fn attach_with<'caches>(caches: &'caches RecordingCaches, ttl: f64) -> UsageAttach<'caches> {
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
    let caches = RecordingCaches::default();
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

#[test]
fn cli_sso_session_ttl_is_ten_minutes() {
    assert_eq!(
        Duration::try_from_secs_f64(CLI_SSO_SESSION_TTL_SECONDS).expect("the TTL is a duration"),
        Duration::from_secs(600)
    );
}

#[tokio::test]
async fn attach_usage_redis_backs_the_auth_cache_when_asked() {
    let redis = standalone_cache().await;
    let caches = RecordingCaches::default();
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
    let caches = RecordingCaches::default();
    attach_usage_redis(&redis, true, &attach_with(&caches, 90.0));
    let held = all_caches(&caches);
    assert_eq!(held.len(), 3);
    assert!(held.iter().all(|cache| Arc::ptr_eq(cache, &redis)));
}

#[tokio::test]
async fn attach_usage_redis_logs_the_enabled_line() {
    let redis = standalone_cache().await;
    let caches = RecordingCaches::default();
    let ((), lines) = capture_logs(|| {
        attach_usage_redis(&redis, true, &attach_with(&caches, 120.0));
    });
    let infos = messages(&lines, Level::INFO);
    assert_eq!(
        infos,
        vec![
            "enable_redis_auth_cache=True: attached Redis to user_api_key_cache \u{2014} virtual-key lookups are now shared across all proxy workers.".to_owned()
        ]
    );
}

#[tokio::test]
async fn attach_usage_redis_logs_the_disabled_line() {
    let redis = standalone_cache().await;
    let caches = RecordingCaches::default();
    let ((), lines) = capture_logs(|| {
        attach_usage_redis(&redis, false, &attach_with(&caches, 120.0));
    });
    let infos = messages(&lines, Level::INFO);
    assert_eq!(
        infos,
        vec![
            "enable_redis_auth_cache is not set: user_api_key_cache remains in-memory only (per-worker). Set litellm_settings.enable_redis_auth_cache: true to share the auth cache across workers and reduce DB load.".to_owned()
        ]
    );
}

#[tokio::test]
async fn usage_cache_target_attaches_to_a_real_dual_cache() {
    let redis = standalone_cache().await;
    let cache = dual_cache(None);
    UsageCacheTarget::attach_redis(&cache, Arc::clone(&redis), Some(30.0));
    let held = cache.redis_cache.read().expect("the client lock");
    assert!(held.as_ref().is_some_and(|held| Arc::ptr_eq(held, &redis)));
    assert_eq!(
        *cache.default_redis_ttl.read().expect("the ttl lock"),
        Some(30.0)
    );
}

#[tokio::test]
async fn config_cache_target_replaces_a_real_dual_cache_client() {
    let original = standalone_cache().await;
    let replacement = standalone_cache().await;
    let cache = dual_cache(Some(Arc::clone(&original)));
    ConfigCacheTarget::replace_redis(&cache, Arc::clone(&replacement));
    let held = cache.redis_cache.read().expect("the client lock");
    assert!(
        held.as_ref()
            .is_some_and(|held| Arc::ptr_eq(held, &replacement))
    );
}

#[tokio::test]
async fn usage_cache_target_attaches_to_a_real_auth_cache() {
    let redis = standalone_cache().await;
    let cache = auth_cache();
    UsageCacheTarget::attach_redis(&cache, Arc::clone(&redis), Some(45.0));
    let held = cache
        .tier("ordinary-key")
        .redis_cache
        .read()
        .expect("the client lock");
    assert!(held.as_ref().is_some_and(|held| Arc::ptr_eq(held, &redis)));
}
