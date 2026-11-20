pub mod error;
pub mod identity;
pub mod local;
pub mod pin;
pub mod shared;
mod tests;

use self::error::AffinityError;
use self::identity::AffinityKey;
use self::local::LocalPins;
use self::pin::{AffinityPin, StoredPin};
use self::shared::SharedFallback;
use crate::routing_state_store::RoutingStateStore;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Claimed {
    pub pin: StoredPin,
    pub fallback: Option<SharedFallback>,
}

// Router.cache as this worker sees it: its pod-local pins always, plus the shared pins when Redis is configured
pub struct AffinityClaims {
    pub local: LocalPins,
    pub shared: Option<RoutingStateStore>,
}

impl AffinityClaims {
    // claim_affinity_pin: the shared first writer wins and is mirrored locally, and a shared failure keeps same-pod affinity
    pub async fn claim(
        &self,
        key: &AffinityKey,
        pin: &AffinityPin,
        ttl_seconds: u32,
        eligible: Option<&[AffinityPin]>,
    ) -> Result<Claimed, AffinityError> {
        let Some(store) = &self.shared else {
            let winner = self
                .local
                .claim_in_memory(key, pin, ttl_seconds, eligible)?;
            return Ok(Claimed {
                pin: winner,
                fallback: None,
            });
        };
        match shared::claim(store, key, pin, ttl_seconds, eligible).await {
            Ok(winner) => {
                self.local.set(key, &winner, ttl_seconds)?;
                Ok(Claimed {
                    pin: winner,
                    fallback: None,
                })
            }
            Err(fallback) => {
                let winner = self
                    .local
                    .claim_in_memory(key, pin, ttl_seconds, eligible)?;
                Ok(Claimed {
                    pin: winner,
                    fallback: Some(fallback),
                })
            }
        }
    }
}
