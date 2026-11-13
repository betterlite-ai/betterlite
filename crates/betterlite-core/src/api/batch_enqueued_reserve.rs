use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::budget_permission_rows::JsonData;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use keys::COUNTER_KEY_PREFIX;
use metadata::resolve_enqueued_scopes;
use store::{BatchEnqueuedReserveError, EnqueuedTokenCounters, ReservationScripts};

pub mod keys;
pub mod metadata;
pub mod provider;
pub mod response;
pub mod store;
mod tests;

pub const BATCH_ENQUEUED_TTL_SECONDS: i64 = 691_200;
pub const BATCH_ENQUEUED_LIMIT_KEY: &str = "batch_enqueued_token_limit";

// LiteLLM spells the ttl as an int constant; it reaches the scripts and the local counters as seconds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchEnqueuedTokenSettings {
    pub ttl_seconds: i64,
}

impl Default for BatchEnqueuedTokenSettings {
    fn default() -> Self {
        Self {
            ttl_seconds: BATCH_ENQUEUED_TTL_SECONDS,
        }
    }
}

// the source's injectable monotonic clock, which stamps every reservation
pub type Monotonic = Arc<dyn Fn() -> f64 + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKey {
    ApiKey,
    Team,
}

impl ScopeKey {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::Team => "team",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchEnqueuedTokenScope {
    pub key: ScopeKey,
    pub value: String,
    pub limit: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationBackend {
    Redis,
    Memory,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BatchEnqueuedTokenReservation {
    pub tokens: i64,
    pub scopes: Vec<BatchEnqueuedTokenScope>,
    pub backend: ReservationBackend,
    pub owner: String,
    pub reserved_at_monotonic: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchEnqueuedTokenOverLimit {
    pub scope: BatchEnqueuedTokenScope,
    pub enqueued: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BatchEnqueuedReserveResult {
    Reservation(BatchEnqueuedTokenReservation),
    OverLimit(BatchEnqueuedTokenOverLimit),
}

// the fields of UserAPIKeyAuth this slice reads; an empty string is absent, as the source skips it
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BatchEnqueuedPrincipal {
    pub api_key: Option<SecretString>,
    pub team_id: Option<String>,
    pub metadata: Option<BTreeMap<String, JsonData>>,
    pub team_metadata: Option<BTreeMap<String, JsonData>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchEnqueuedReserveInput {
    pub tokens: i64,
    pub principal: BatchEnqueuedPrincipal,
}

pub struct BatchEnqueuedTokenStore<Scripts, Counters> {
    pub clock: Monotonic,
    pub scripts: Option<Scripts>,
    pub counters: Arc<Mutex<Counters>>,
    pub owner: String,
    pub settings: BatchEnqueuedTokenSettings,
}

impl<Scripts, Counters> BatchEnqueuedTokenStore<Scripts, Counters>
where
    Scripts: ReservationScripts,
    Counters: EnqueuedTokenCounters,
{
    pub async fn reserve(
        &self,
        input: &BatchEnqueuedReserveInput,
    ) -> Result<BatchEnqueuedReserveResult, BatchEnqueuedReserveError> {
        self.reserve_scopes(input.tokens, &resolve_enqueued_scopes(&input.principal))
            .await
    }

    pub async fn reserve_scopes(
        &self,
        tokens: i64,
        scopes: &[BatchEnqueuedTokenScope],
    ) -> Result<BatchEnqueuedReserveResult, BatchEnqueuedReserveError> {
        if tokens <= 0 || scopes.is_empty() {
            return Ok(BatchEnqueuedReserveResult::Reservation(
                BatchEnqueuedTokenReservation {
                    tokens: tokens.max(0),
                    scopes: scopes.to_vec(),
                    backend: ReservationBackend::Redis,
                    owner: String::new(),
                    reserved_at_monotonic: (self.clock)(),
                },
            ));
        }
        let keys: Vec<String> = scopes
            .iter()
            .map(|scope| format!("{COUNTER_KEY_PREFIX}{}:{}", scope.key.as_str(), scope.value))
            .collect();
        if let Some(scripts) = &self.scripts {
            match self.reserve_via_redis(scripts, tokens, scopes, &keys).await {
                Ok(outcome) => return Ok(outcome),
                Err(error) => tracing::warn!(
                    error = %error,
                    "redis enqueued-token reserve failed; falling back to in-memory counters"
                ),
            }
        }
        self.reserve_in_memory(tokens, scopes, &keys)
    }

    fn reserve_in_memory(
        &self,
        tokens: i64,
        scopes: &[BatchEnqueuedTokenScope],
        keys: &[String],
    ) -> Result<BatchEnqueuedReserveResult, BatchEnqueuedReserveError> {
        let started = (self.clock)();
        let mut counters = self.counters.lock().unwrap_or_else(PoisonError::into_inner);
        let mut currents = Vec::with_capacity(scopes.len());
        for key in keys {
            let stored = counters.get(key)?;
            currents.push(stored.unwrap_or(0));
        }
        let mut updated = Vec::with_capacity(scopes.len());
        for (scope, current) in scopes.iter().zip(&currents) {
            match current.checked_add(tokens) {
                Some(next) if next <= scope.limit => updated.push(next),
                // an i64 overflow puts the sum past every limit
                None | Some(_) => {
                    return Ok(BatchEnqueuedReserveResult::OverLimit(
                        BatchEnqueuedTokenOverLimit {
                            scope: scope.clone(),
                            enqueued: *current,
                        },
                    ));
                }
            }
        }
        for (key, next) in keys.iter().zip(updated) {
            counters.set(key, next, self.settings.ttl_seconds)?;
        }
        drop(counters);
        Ok(BatchEnqueuedReserveResult::Reservation(
            BatchEnqueuedTokenReservation {
                tokens,
                scopes: scopes.to_vec(),
                backend: ReservationBackend::Memory,
                owner: self.owner.clone(),
                reserved_at_monotonic: started,
            },
        ))
    }

    async fn reserve_via_redis(
        &self,
        scripts: &Scripts,
        tokens: i64,
        scopes: &[BatchEnqueuedTokenScope],
        keys: &[String],
    ) -> Result<BatchEnqueuedReserveResult, Scripts::Error> {
        let started = (self.clock)();
        let mut reserved = Vec::with_capacity(keys.len());
        for (scope, key) in scopes.iter().zip(keys) {
            let reply = match scripts
                .reserve(key, tokens, self.settings.ttl_seconds, scope.limit)
                .await
            {
                Ok(reply) => reply,
                Err(error) => {
                    rollback_partial_reserve(scripts, tokens, &reserved).await;
                    return Err(error);
                }
            };
            if reply.status != 1 {
                rollback_partial_reserve(scripts, tokens, &reserved).await;
                return Ok(BatchEnqueuedReserveResult::OverLimit(
                    BatchEnqueuedTokenOverLimit {
                        scope: scope.clone(),
                        enqueued: reply.count,
                    },
                ));
            }
            reserved.push(key.clone());
        }
        Ok(BatchEnqueuedReserveResult::Reservation(
            BatchEnqueuedTokenReservation {
                tokens,
                scopes: scopes.to_vec(),
                backend: ReservationBackend::Redis,
                owner: String::new(),
                reserved_at_monotonic: started,
            },
        ))
    }
}

async fn rollback_partial_reserve<Scripts>(scripts: &Scripts, tokens: i64, keys: &[String])
where
    Scripts: ReservationScripts,
{
    for key in keys {
        if let Err(error) = scripts.refund(key, tokens).await {
            tracing::warn!(
                error = %error,
                "rollback of partially reserved enqueued tokens failed; the leaked increments expire with the ttl"
            );
            return;
        }
    }
}
