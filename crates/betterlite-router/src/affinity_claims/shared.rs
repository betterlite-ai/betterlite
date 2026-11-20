use super::identity::AffinityKey;
use super::pin::{AffinityPin, StoredPin};
use crate::routing_state_store::RoutingStateStore;
use crate::routing_value_store::{PinValue, RouteValue, StoreRead};

// a claim that keeps losing the compare-and-set race falls back to the pod-local claim instead of spinning
pub const MAX_CLAIM_ATTEMPTS: u8 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedFallback {
    Unavailable,
    Conflict,
    ValueUnusable,
}

enum Decision {
    Keep {
        value: RouteValue,
        stored: StoredPin,
    },
    Replace,
    Return(StoredPin),
}

// the Redis claim script without Lua: read the shared pin, decide as the script does, and write it back only when the read still holds
pub(super) async fn claim(
    store: &RoutingStateStore,
    key: &AffinityKey,
    pin: &AffinityPin,
    ttl_seconds: u32,
    eligible: Option<&[AffinityPin]>,
) -> Result<StoredPin, SharedFallback> {
    let ttl = Some(f64::from(ttl_seconds));
    let requested = serde_json::to_string(&pin.0).unwrap_or_default();
    let ours = RouteValue::Pin(PinValue {
        model_id: requested.clone(),
    });
    let mut current = read(store, key).await?;
    for _attempt in 0..MAX_CLAIM_ATTEMPTS {
        match evaluate(&current, pin, &requested, eligible)? {
            Decision::Return(stored) => return Ok(stored),
            Decision::Keep { value, stored } => {
                let outcome = store
                    .compare_and_set(&key.0, &current, &value, ttl)
                    .await
                    .map_err(|_error| SharedFallback::Unavailable)?;
                if outcome.value.swapped {
                    return Ok(stored);
                }
                current = outcome.value.current;
            }
            Decision::Replace => {
                let outcome = store
                    .compare_and_set(&key.0, &current, &ours, ttl)
                    .await
                    .map_err(|_error| SharedFallback::Unavailable)?;
                if outcome.value.swapped {
                    return Ok(StoredPin::Pin(pin.clone()));
                }
                current = outcome.value.current;
            }
        }
    }
    Err(SharedFallback::Conflict)
}

async fn read(store: &RoutingStateStore, key: &AffinityKey) -> Result<StoreRead, SharedFallback> {
    let observed = store
        .get(&key.0)
        .await
        .map_err(|_error| SharedFallback::Unavailable)?;
    if observed.warning.is_some() {
        return Err(SharedFallback::Unavailable);
    }
    Ok(observed.value)
}

fn evaluate(
    current: &StoreRead,
    pin: &AffinityPin,
    requested: &str,
    eligible: Option<&[AffinityPin]>,
) -> Result<Decision, SharedFallback> {
    let StoreRead::Value(value) = current else {
        return Ok(Decision::Replace);
    };
    let RouteValue::Pin(stored_value) = value else {
        return Err(SharedFallback::ValueUnusable);
    };
    let stored = StoredPin::parse(stored_value.model_id.as_bytes())
        .map_err(|_error| SharedFallback::ValueUnusable)?;
    match eligible {
        Some(candidates) => {
            if candidates
                .iter()
                .any(|candidate| stored.matches_exactly(candidate))
            {
                Ok(Decision::Keep {
                    value: value.clone(),
                    stored,
                })
            } else {
                Ok(Decision::Replace)
            }
        }
        None => {
            if stored.wire_text() == requested {
                Ok(Decision::Keep {
                    value: value.clone(),
                    stored,
                })
            } else {
                Ok(Decision::Return(stored))
            }
        }
    }
}
