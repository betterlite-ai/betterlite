use std::collections::BTreeSet;
use std::sync::Arc;

use crate::routing_value_store::RouteKey;

use super::local::{LocalTier, PreparedRead, Reservation, push_unique};
use super::{ReadTiers, RequestReads, RoutingPrefetch, RoutingReadKeys, TierReservation};

impl<V> RoutingPrefetch<V>
where
    V: Clone,
{
    pub fn arm(
        keys: &RoutingReadKeys,
        tiers: &ReadTiers<V>,
        remote_available: bool,
        request: &mut RequestReads<V>,
    ) -> bool {
        if !remote_available || request.prefetch.is_some() {
            return false;
        }
        let cooldown = tiers.cooldown.prepare(&keys.cooldown);
        let usage = usage_read(keys, tiers);
        let fetched = merged_due(&cooldown.reservation, usage.as_ref().map(|(_, read)| read));
        if fetched.is_empty() {
            return false;
        }
        request.prefetch = Some(Self {
            keys: declared_keys(keys),
            fetched,
            reservations: reservations(tiers, cooldown.reservation, usage),
        });
        true
    }
}

fn usage_read<V>(
    keys: &RoutingReadKeys,
    tiers: &ReadTiers<V>,
) -> Option<(Arc<LocalTier<V>>, PreparedRead<V>)>
where
    V: Clone,
{
    let (Some(usage_keys), Some(tier)) = (&keys.usage, tiers.usage.as_ref()) else {
        return None;
    };
    let combined = usage_keys.combined();
    Some((Arc::clone(tier), tier.prepare(&combined)))
}

fn merged_due<V>(cooldown: &Reservation, usage: Option<&PreparedRead<V>>) -> Vec<RouteKey> {
    let mut fetched = Vec::new();
    for key in &cooldown.due {
        push_unique(&mut fetched, key);
    }
    if let Some(read) = usage {
        for key in &read.reservation.due {
            push_unique(&mut fetched, key);
        }
    }
    fetched
}

fn declared_keys(keys: &RoutingReadKeys) -> BTreeSet<RouteKey> {
    let mut declared: BTreeSet<_> = keys.cooldown.iter().cloned().collect();
    if let Some(usage_keys) = &keys.usage {
        declared.extend(usage_keys.combined());
    }
    declared
}

fn reservations<V>(
    tiers: &ReadTiers<V>,
    cooldown: Reservation,
    usage: Option<(Arc<LocalTier<V>>, PreparedRead<V>)>,
) -> Vec<TierReservation<V>> {
    let mut held = vec![TierReservation {
        tier: Arc::clone(&tiers.cooldown),
        reservation: cooldown,
    }];
    if let Some((tier, read)) = usage {
        held.push(TierReservation {
            tier,
            reservation: read.reservation,
        });
    }
    held
}
