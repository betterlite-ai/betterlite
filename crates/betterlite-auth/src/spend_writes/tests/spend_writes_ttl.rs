use crate::spend_writes::error::LayersError;
use crate::spend_writes::layers::CounterLayers as _;
use crate::spend_writes::ttl::refresh_spend_counter_ttl;

use super::refusing::refusing_redis_cache;
use super::support::{Fault, KEY, Logs, dual_cache, fixture};

#[tokio::test]
async fn a_successful_refresh_reports_true() {
    let fx = fixture(true);
    fx.layers.refresh(true);
    let refreshed = refresh_spend_counter_ttl(&fx.writes, KEY).await;
    assert!(refreshed);
    assert!(fx.journal.records(&format!("refresh_ttl:{KEY}")));
}

#[tokio::test]
async fn an_unsuccessful_refresh_reports_false() {
    let fx = fixture(true);
    fx.layers.refresh(false);
    let refreshed = refresh_spend_counter_ttl(&fx.writes, KEY).await;
    assert!(!refreshed);
}

#[tokio::test]
async fn a_refresh_failure_reports_false_and_logs_debug() {
    let logs = Logs::capture();
    let fx = fixture(true);
    fx.layers.faults.add(Fault::RefreshTtl);
    let refreshed = refresh_spend_counter_ttl(&fx.writes, KEY).await;
    assert!(!refreshed);
    assert!(logs.contains("spend counter TTL refresh skipped"));
    assert!(logs.warnings().is_empty());
}

#[tokio::test]
async fn no_redis_tier_refreshes_nothing() {
    let fx = fixture(false);
    fx.layers.refresh(true);
    let refreshed = refresh_spend_counter_ttl(&fx.writes, KEY).await;
    assert!(!refreshed);
    assert_eq!(fx.journal.count("refresh_ttl:"), 0);
}

#[tokio::test]
async fn the_real_tier_reports_a_missing_redis_tier() {
    let cache = dual_cache(None);
    let error = cache
        .refresh_redis_ttl(KEY)
        .await
        .expect_err("the missing tier answers");
    assert!(matches!(error, LayersError::Unavailable));
}

#[tokio::test]
async fn the_real_tier_reports_a_refused_refresh() {
    let cache = dual_cache(Some(refusing_redis_cache().await));
    let error = cache
        .refresh_redis_ttl(KEY)
        .await
        .expect_err("the breaker refuses the refresh");
    assert!(matches!(error, LayersError::Redis(_)));
}
