use std::collections::BTreeMap;
use std::sync::Arc;

use crate::routing_value_store::RouteKey;

use super::local::{LocalTier, PreparedRead, push_unique};
use super::store::{BatchReadStore, ReadWarning};
use super::{
    PrefetchedUsage, ReadTiers, RoutingPrefetch, RoutingReadBatch, RoutingReadResults,
    RoutingReadSession, UsageReadKeys,
};

type Groups<V> = (Vec<Option<V>>, Option<Vec<Option<V>>>, Option<ReadWarning>);
type UsagePrepared<V> = (Arc<LocalTier<V>>, PreparedRead<V>);

impl<V> RoutingReadBatch<V>
where
    V: Clone + PartialEq + Send + Sync,
{
    pub async fn read_routing_state<S>(
        &mut self,
        session: &mut RoutingReadSession<'_, S, V>,
        cooldown_keys: &[RouteKey],
    ) -> RoutingReadResults<V>
    where
        S: BatchReadStore<Value = V> + Sync,
    {
        let usage_keys = self.usage.as_ref().map(UsageReadKeys::combined);
        let mut needed = Vec::with_capacity(cooldown_keys.len());
        needed.extend(cooldown_keys.iter().cloned());
        if let Some(keys) = &usage_keys {
            needed.extend(keys.iter().cloned());
        }
        let prefetched = match session.request.take(&needed) {
            Some(prefetch) => {
                consume_prefetch(prefetch, session, cooldown_keys, usage_keys.as_deref()).await
            }
            None => None,
        };
        let (cooldown, usage, warning) = match prefetched {
            Some(groups) => groups,
            None => {
                fresh_read(
                    session.store,
                    session.tiers,
                    cooldown_keys,
                    usage_keys.as_deref(),
                )
                .await
            }
        };
        if let (Some(keys), Some(results)) = (&usage_keys, &usage) {
            let mapped = keys.iter().cloned().zip(results.iter().cloned()).collect();
            self.prefetched_usage = Some(PrefetchedUsage {
                keys: keys.clone(),
                values: Some(mapped),
            });
        }
        RoutingReadResults {
            cooldown,
            usage,
            warning,
        }
    }
}

async fn consume_prefetch<S, V>(
    mut prefetch: RoutingPrefetch<V>,
    session: &RoutingReadSession<'_, S, V>,
    cooldown_keys: &[RouteKey],
    usage_keys: Option<&[RouteKey]>,
) -> Option<Groups<V>>
where
    S: BatchReadStore<Value = V> + Sync,
    V: Clone + PartialEq + Send + Sync,
{
    let outcome = session.store.batch_read(&prefetch.fetched).await;
    if outcome.warning.is_some() {
        return None;
    }
    let values: BTreeMap<RouteKey, V> = prefetch
        .fetched
        .iter()
        .cloned()
        .zip(outcome.values)
        .filter_map(|(key, stored)| stored.map(|remote| (key, remote)))
        .collect();
    for entry in &prefetch.reservations {
        entry.tier.backfill(&entry.reservation.due, &values);
    }
    prefetch.reservations.clear();
    let cooldown = apply_prefetched(
        &session.tiers.cooldown,
        cooldown_keys,
        &prefetch.fetched,
        &values,
    )?;
    let usage = match (usage_keys, session.tiers.usage.as_ref()) {
        (Some(keys), Some(tier)) => Some(apply_prefetched(tier, keys, &prefetch.fetched, &values)?),
        _ => None,
    };
    Some((cooldown, usage, None))
}

pub(super) fn apply_prefetched<V>(
    tier: &LocalTier<V>,
    keys: &[RouteKey],
    fetched: &[RouteKey],
    values: &BTreeMap<RouteKey, V>,
) -> Option<Vec<Option<V>>>
where
    V: Clone,
{
    let local = tier.batch_get(keys);
    let uncovered = keys.iter().zip(&local).any(|(key, value)| {
        value.is_none() && !fetched.iter().any(|fetched_key| fetched_key == key)
    });
    if uncovered {
        return None;
    }
    let mut missed = BTreeMap::new();
    for (key, value) in keys.iter().zip(&local) {
        if value.is_none()
            && let Some(remote) = values.get(key)
        {
            missed.insert(key.clone(), remote.clone());
        }
    }
    Some(tier.apply(keys, &local, &missed))
}

async fn fresh_read<S, V>(
    store: &S,
    tiers: &ReadTiers<V>,
    cooldown_keys: &[RouteKey],
    usage_keys: Option<&[RouteKey]>,
) -> Groups<V>
where
    S: BatchReadStore<Value = V> + Sync,
    V: Clone + PartialEq + Send + Sync,
{
    let cooldown = tiers.cooldown.prepare(cooldown_keys);
    let usage = prepare_usage(tiers, usage_keys);
    let due = due_keys(&cooldown, usage.as_ref());
    if due.is_empty() {
        return (cooldown.local, usage.map(|(_, read)| read.local), None);
    }
    let outcome = store.batch_read(&due).await;
    if let Some(warning) = outcome.warning {
        return warning_groups(warning, tiers, cooldown, usage);
    }
    let remote: BTreeMap<RouteKey, V> = due
        .iter()
        .cloned()
        .zip(outcome.values)
        .filter_map(|(key, stored)| stored.map(|remote| (key, remote)))
        .collect();
    let cooldown_values = apply_due(&tiers.cooldown, cooldown_keys, &cooldown, &remote);
    let usage_values = match (usage_keys, usage) {
        (Some(keys), Some((tier, read))) => Some(apply_due(&tier, keys, &read, &remote)),
        _ => None,
    };
    (cooldown_values, usage_values, None)
}

fn warning_groups<V>(
    warning: ReadWarning,
    tiers: &ReadTiers<V>,
    cooldown: PreparedRead<V>,
    usage: Option<UsagePrepared<V>>,
) -> Groups<V>
where
    V: Clone,
{
    if warning == ReadWarning::CircuitOpen {
        tiers.cooldown.release(&cooldown.reservation);
        if let Some((tier, read)) = &usage {
            tier.release(&read.reservation);
        }
    }
    (
        cooldown.local,
        usage.map(|(_, read)| read.local),
        Some(warning),
    )
}

fn prepare_usage<V>(
    tiers: &ReadTiers<V>,
    usage_keys: Option<&[RouteKey]>,
) -> Option<UsagePrepared<V>>
where
    V: Clone,
{
    let (Some(keys), Some(tier)) = (usage_keys, tiers.usage.as_ref()) else {
        return None;
    };
    Some((Arc::clone(tier), tier.prepare(keys)))
}

fn due_keys<V>(cooldown: &PreparedRead<V>, usage: Option<&UsagePrepared<V>>) -> Vec<RouteKey> {
    let mut due = Vec::new();
    for key in &cooldown.reservation.due {
        push_unique(&mut due, key);
    }
    if let Some((_, read)) = usage {
        for key in &read.reservation.due {
            push_unique(&mut due, key);
        }
    }
    due
}

fn apply_due<V>(
    tier: &LocalTier<V>,
    keys: &[RouteKey],
    prepared: &PreparedRead<V>,
    remote: &BTreeMap<RouteKey, V>,
) -> Vec<Option<V>>
where
    V: Clone,
{
    let mut fetched = BTreeMap::new();
    for key in &prepared.reservation.due {
        if let Some(value) = remote.get(key) {
            fetched.insert(key.clone(), value.clone());
        }
    }
    tier.apply(keys, &prepared.local, &fetched)
}
