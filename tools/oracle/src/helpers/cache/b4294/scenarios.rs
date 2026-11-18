use serde_json::json;

use super::input::Fields;
use super::scenario::{
    EVERY, Failures, HarnessMemory, Setup, THROTTLED, build_cache, group, reservation, seeded,
    ttl_values,
};
use betterlite_cache::batch_reads::BatchReads;
use betterlite_cache::dual_cache::{DualCache, Ttl};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::HelperResult;

pub(crate) async fn reserve(call: &Fields) -> HelperResult {
    let setup = Setup::start(call).await?;
    let cache = seeded(call, Some(&setup.shared))?;
    reserve_output(call, &setup, &cache)
}

pub(crate) async fn sync_backfill(call: &Fields) -> HelperResult {
    let setup = Setup::start(call).await?;
    let cache = seeded(call, Some(&setup.shared))?;
    backfill_output(call, &setup, &cache).await
}

pub(crate) async fn sync_errors(call: &Fields) -> HelperResult {
    let setup = Setup::start(call).await?;
    let rejecting = seeded(call, Some(&setup.rejecting))?;
    errors_output(call, &setup, &rejecting)
}

pub(crate) async fn async_switch(call: &Fields) -> HelperResult {
    let setup = Setup::start(call).await?;
    let cache = seeded(call, Some(&setup.shared))?;
    switch_output(call, &setup, &cache).await
}

pub(crate) async fn shared_merge(call: &Fields) -> HelperResult {
    let setup = Setup::start(call).await?;
    let (caches, results) = group(&setup, &call.caches, call).await?;
    merge_output(call, &setup, &caches, results).await
}

pub(crate) async fn shared_failures(call: &Fields) -> HelperResult {
    let setup = Setup::start(call).await?;
    let (breaker_caches, breaker) = group(&setup, &call.breaker_caches, call).await?;
    failures_output(call, &setup, &breaker_caches, breaker).await
}

fn reserve_output(call: &Fields, setup: &Setup, cache: &DualCache<HarnessMemory>) -> HelperResult {
    let first = cache.reserve_redis_batch_reads(&call.keys);
    let second = cache.reserve_redis_batch_reads(&call.keys);
    let read = cache.batch_get_cache(&call.read_keys, false);
    let mut observed = call.keys.clone();
    observed.extend(call.read_keys.iter().cloned());
    let after_read = cache.reserve_redis_batch_reads(&observed);
    let rejecting = seeded(call, Some(&setup.rejecting))?;
    let failed = rejecting.batch_get_cache(&call.due_keys, false);
    let absent = seeded(call, None)?;
    Ok(json!({
        "first": reservation(first),
        "second": reservation(second),
        "read": read,
        "after_read": reservation(after_read),
        "failed": failed,
        "after_failure": reservation(rejecting.reserve_redis_batch_reads(&call.due_keys)),
        "no_redis": reservation(absent.reserve_redis_batch_reads(&call.keys)),
        "mgets": setup.shared_server.mgets(),
    }))
}

async fn backfill_output(
    call: &Fields,
    setup: &Setup,
    cache: &DualCache<HarnessMemory>,
) -> HelperResult {
    let read = cache.batch_get_cache(&call.read_keys, false);
    let mut memory: Vec<Option<Json>> = Vec::with_capacity(call.read_keys.len());
    for key in &call.read_keys {
        memory.push(cache.get_cache(key, true));
    }
    let ttls = ttl_values(cache, &call.ttl_keys).await?;
    let all_hit = cache.batch_get_cache(&call.keys, false);
    let local_only = cache.batch_get_cache(&call.read_keys, true);
    let absent = seeded(call, None)?;
    Ok(json!({
        "read": read,
        "memory": memory,
        "ttls": ttls,
        "all_hit": all_hit,
        "local_only": local_only,
        "no_redis": absent.batch_get_cache(&call.read_keys, false),
        "mgets": setup.shared_server.mgets(),
    }))
}

fn errors_output(
    call: &Fields,
    setup: &Setup,
    rejecting: &DualCache<HarnessMemory>,
) -> HelperResult {
    let breaker_read = rejecting.batch_get_cache(&call.read_keys, false);
    let breaker_due = rejecting.reserve_redis_batch_reads(&call.due_keys);
    let memory_failure = build_cache(
        Some(&setup.shared),
        call,
        Failures {
            reads: true,
            writes: false,
        },
    )?
    .batch_get_cache(&call.read_keys, false);
    let write_failure = build_cache(
        Some(&setup.shared),
        call,
        Failures {
            reads: false,
            writes: true,
        },
    )?
    .batch_get_cache(&call.write_keys, false);
    Ok(json!({
        "breaker_read": breaker_read,
        "breaker_due": reservation(breaker_due),
        "memory_failure": memory_failure,
        "write_failure": write_failure,
        "mgets": setup.shared_server.mgets(),
    }))
}

async fn switch_output(
    call: &Fields,
    setup: &Setup,
    cache: &DualCache<HarnessMemory>,
) -> HelperResult {
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
    let memory: Vec<Option<Json>> = observed
        .iter()
        .map(|key| cache.get_cache(key, true))
        .collect();
    Ok(json!({
        "first": first,
        "second": second,
        "third": third,
        "duplicates": duplicates,
        "merge": merge,
        "memory": memory,
        "mgets": setup.shared_server.mgets(),
    }))
}

async fn merge_output(
    call: &Fields,
    setup: &Setup,
    caches: &[DualCache<HarnessMemory>],
    results: Vec<Option<Vec<Option<Json>>>>,
) -> HelperResult {
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

async fn failures_output(
    call: &Fields,
    setup: &Setup,
    breaker_caches: &[DualCache<HarnessMemory>],
    breaker: Vec<Option<Vec<Option<Json>>>>,
) -> HelperResult {
    let mut breaker_due: Vec<serde_json::Value> = Vec::with_capacity(breaker_caches.len());
    for (built, entry) in breaker_caches.iter().zip(&call.breaker_caches) {
        breaker_due.push(reservation(built.reserve_redis_batch_reads(&entry.keys)));
    }
    let (_prepare_caches, prepare) = group(setup, &call.prepare_caches, call).await?;
    let (_apply_caches, apply) = group(setup, &call.apply_caches, call).await?;
    Ok(json!({
        "breaker": breaker,
        "breaker_due": breaker_due,
        "prepare": prepare,
        "apply": apply,
        "mgets": setup.shared_server.mgets(),
    }))
}
