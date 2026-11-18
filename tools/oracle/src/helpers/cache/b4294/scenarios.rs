use std::collections::BTreeMap;

use serde_json::json;

use super::input::Fields;
use super::scenario::{
    EVERY, Failures, Setup, THROTTLED, build_cache, group, reservation, seed, ttl_values,
};
use betterlite_cache::batch_reads::BatchReads;
use betterlite_cache::dual_cache::Ttl;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::HelperResult;

pub(crate) async fn reserve(call: &Fields) -> HelperResult {
    let setup = Setup::start(call).await?;
    let cache = build_cache(Some(&setup.shared), call, Failures::default())?;
    seed(&cache, &call.memory);
    let first = cache.reserve_redis_batch_reads(&call.keys);
    let second = cache.reserve_redis_batch_reads(&call.keys);
    let read = cache.batch_get_cache(&call.read_keys, false);
    let mut observed = call.keys.clone();
    observed.extend(call.read_keys.iter().cloned());
    let after_read = cache.reserve_redis_batch_reads(&observed);
    let rejecting = build_cache(Some(&setup.rejecting), call, Failures::default())?;
    seed(&rejecting, &call.memory);
    let failed = rejecting.batch_get_cache(&call.due_keys, false);
    let after_failure = rejecting.reserve_redis_batch_reads(&call.due_keys);
    let no_redis = build_cache(None, call, Failures::default())?;
    seed(&no_redis, &call.memory);
    let absent = no_redis.reserve_redis_batch_reads(&call.keys);
    Ok(json!({
        "first": reservation(first),
        "second": reservation(second),
        "read": read,
        "after_read": reservation(after_read),
        "failed": failed,
        "after_failure": reservation(after_failure),
        "no_redis": reservation(absent),
        "mgets": setup.shared_server.mgets(),
    }))
}

pub(crate) async fn sync_backfill(call: &Fields) -> HelperResult {
    let setup = Setup::start(call).await?;
    let cache = build_cache(Some(&setup.shared), call, Failures::default())?;
    seed(&cache, &call.memory);
    let read = cache.batch_get_cache(&call.read_keys, false);
    let stored: Vec<Option<Json>> = call
        .read_keys
        .iter()
        .map(|key| cache.get_cache(key, true))
        .collect();
    let ttls = ttl_values(&cache, &call.ttl_keys).await?;
    let all_hit = cache.batch_get_cache(&call.keys, false);
    let local_only = cache.batch_get_cache(&call.read_keys, true);
    let no_redis = build_cache(None, call, Failures::default())?;
    seed(&no_redis, &call.memory);
    let absent = no_redis.batch_get_cache(&call.read_keys, false);
    Ok(json!({
        "read": read,
        "memory": stored,
        "ttls": ttls,
        "all_hit": all_hit,
        "local_only": local_only,
        "no_redis": absent,
        "mgets": setup.shared_server.mgets(),
    }))
}

pub(crate) async fn sync_errors(call: &Fields) -> HelperResult {
    let setup = Setup::start(call).await?;
    let rejecting = build_cache(Some(&setup.rejecting), call, Failures::default())?;
    seed(&rejecting, &call.memory);
    let breaker_read = rejecting.batch_get_cache(&call.read_keys, false);
    let breaker_due = rejecting.reserve_redis_batch_reads(&call.due_keys);
    let failing_read = build_cache(
        Some(&setup.shared),
        call,
        Failures {
            reads: true,
            writes: false,
        },
    )?;
    let memory_failure = failing_read.batch_get_cache(&call.read_keys, false);
    let failing_write = build_cache(
        Some(&setup.shared),
        call,
        Failures {
            reads: false,
            writes: true,
        },
    )?;
    let write_failure = failing_write.batch_get_cache(&call.write_keys, false);
    Ok(json!({
        "breaker_read": breaker_read,
        "breaker_due": reservation(breaker_due),
        "memory_failure": memory_failure,
        "write_failure": write_failure,
        "mgets": setup.shared_server.mgets(),
    }))
}

pub(crate) async fn async_switch(call: &Fields) -> HelperResult {
    let setup = Setup::start(call).await?;
    let cache = build_cache(Some(&setup.shared), call, Failures::default())?;
    seed(&cache, &call.memory);
    let first = cache.async_batch_get_cache(&call.keys, THROTTLED).await;
    let second = cache.async_batch_get_cache(&call.keys, THROTTLED).await;
    let third = cache.async_batch_get_cache(&call.keys, EVERY).await;
    let duplicates = cache
        .async_batch_get_cache(&call.duplicate_keys, EVERY)
        .await;
    let merge = cache.async_batch_get_cache(&call.merge_keys, EVERY).await;
    let mut observed = call.keys.clone();
    observed.extend(call.merge_keys.iter().cloned());
    observed.extend(call.duplicate_keys.iter().cloned());
    let stored: Vec<Option<Json>> = observed
        .iter()
        .map(|key| cache.get_cache(key, true))
        .collect();
    Ok(json!({
        "first": first,
        "second": second,
        "third": third,
        "duplicates": duplicates,
        "merge": merge,
        "memory": stored,
        "mgets": setup.shared_server.mgets(),
    }))
}

pub(crate) async fn shared_merge(call: &Fields) -> HelperResult {
    let setup = Setup::start(call).await?;
    let (caches, results) = group(&setup, &call.caches, call).await?;
    let mut memory: Vec<Vec<Option<Json>>> = Vec::with_capacity(caches.len());
    let mut ttls: Vec<Vec<Option<Ttl>>> = Vec::with_capacity(caches.len());
    for (built, entry) in caches.iter().zip(&call.caches) {
        let values = entry
            .keys
            .iter()
            .map(|key| built.get_cache(key, true))
            .collect();
        memory.push(values);
        ttls.push(ttl_values(built, &entry.keys).await?);
    }
    Ok(json!({
        "reads": results,
        "memory": memory,
        "ttls": ttls,
        "shared_mgets": setup.shared_server.mgets(),
        "own_mgets": setup.own_server.mgets(),
    }))
}

pub(crate) async fn shared_failures(call: &Fields) -> HelperResult {
    let setup = Setup::start(call).await?;
    let (breaker_caches, breaker) = group(&setup, &call.breaker_caches, call).await?;
    let breaker_due: Vec<serde_json::Value> = breaker_caches
        .iter()
        .zip(&call.breaker_caches)
        .map(|(built, entry)| reservation(built.reserve_redis_batch_reads(&entry.keys)))
        .collect();
    let (_prepare_caches, prepare) = group(&setup, &call.prepare_caches, call).await?;
    let (_apply_caches, apply) = group(&setup, &call.apply_caches, call).await?;
    Ok(json!({
        "breaker": breaker,
        "breaker_due": breaker_due,
        "prepare": prepare,
        "apply": apply,
        "mgets": setup.shared_server.mgets(),
    }))
}
