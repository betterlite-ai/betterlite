use std::sync::{Arc, Mutex, PoisonError};

use betterlite_redact::structured::SensitiveDataMasker;

use crate::cooldown_state::record::{
    CooldownRecord, KEY_PREFIX, KEY_SUFFIX, MAX_CORRECTED_IN_MEMORY_TTL_SECONDS,
};
use crate::cooldown_state::settings::Settings;
use crate::cooldown_state::shared::{SharedCooldowns, SharedHandle};
use crate::routing_read_batch::local::{
    DEFAULT_LOCAL_TTL_SECONDS, DEFAULT_MAX_LAST_READS, DEFAULT_MAX_LOCAL_ENTRIES, LocalEntry,
    LocalTier, ReadTierSettings, TierState,
};
use crate::routing_read_batch::{ReadTiers, RequestReads, RoutingReadBatch, RoutingReadSession};
use crate::routing_value_store::{Clock, RouteKey, StoreError};

// one cooldown that is still running: the deployment, the seconds it has left, and the entry that says when it ends
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveCooldown {
    pub model_id: String,
    pub remaining_seconds: f64,
    pub record: CooldownRecord,
}

// CooldownCache: the cooldown entries of every deployment, in a throttled memory tier over the shared one
pub struct CooldownCache {
    tier: Arc<LocalTier<CooldownRecord>>,
    shared: Arc<dyn SharedCooldowns>,
    default_cooldown_time: f64,
    clock: Clock,
    masker: SensitiveDataMasker,
}

impl CooldownCache {
    pub fn new(settings: &Settings, shared: Arc<dyn SharedCooldowns>, clock: Clock) -> Self {
        let mut masker = SensitiveDataMasker::with_patterns(Vec::<String>::new(), ["cost"]);
        masker.visible_prefix = 50;
        masker.visible_suffix = 0;
        masker.mask_short_values = false;
        Self {
            tier: Arc::new(LocalTier {
                settings: ReadTierSettings {
                    backfill_ttl_seconds: DEFAULT_LOCAL_TTL_SECONDS,
                    read_expiry_seconds: settings.redis_read_interval_seconds,
                    max_entries: DEFAULT_MAX_LOCAL_ENTRIES,
                    max_last_reads: DEFAULT_MAX_LAST_READS,
                },
                clock: Arc::clone(&clock),
                state: Mutex::new(TierState::default()),
            }),
            shared,
            default_cooldown_time: settings.default_cooldown_time,
            clock,
            masker,
        }
    }

    // add_deployment_to_cooldown: the entry lands in both tiers with a TTL equal to its cooldown time
    pub fn add(
        &self,
        model_id: &str,
        exception: &str,
        status_code: i64,
        cooldown_time: Option<f64>,
    ) -> Result<(), StoreError> {
        let ttl = cooldown_time.unwrap_or(self.default_cooldown_time);
        let key = RouteKey::parse(&format!("{KEY_PREFIX}{model_id}{KEY_SUFFIX}"))?;
        let record = CooldownRecord::new(exception, status_code, (self.clock)(), ttl, &self.masker);
        let expires_at = ttl.mul_add(1.0_f64, (self.tier.clock)());
        let mut state = self
            .tier
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.entries.insert(
            key.clone(),
            LocalEntry {
                value: record.clone(),
                expires_at,
            },
        );
        drop(state);
        self.shared.write(&key, &record, ttl);
        Ok(())
    }

    // async_batch_get_cache over the cooldown keys, through B382's read batch so one request reads the shared tier once
    pub async fn read(
        &self,
        batch: &mut RoutingReadBatch<CooldownRecord>,
        request: &mut RequestReads<CooldownRecord>,
        model_ids: &[String],
    ) -> Result<Vec<Option<CooldownRecord>>, StoreError> {
        let mut keys = Vec::with_capacity(model_ids.len());
        for model_id in model_ids {
            keys.push(RouteKey::parse(&format!(
                "{KEY_PREFIX}{model_id}{KEY_SUFFIX}"
            ))?);
        }
        let tiers = ReadTiers {
            cooldown: Arc::clone(&self.tier),
            usage: None,
        };
        let store = SharedHandle(Arc::clone(&self.shared));
        let mut session = RoutingReadSession {
            store: &store,
            tiers: &tiers,
            request,
        };
        let results = batch.read_routing_state(&mut session, &keys).await;
        if let Some(warning) = results.warning {
            tracing::warn!(
                ?warning,
                "the shared cooldown read failed, so only local entries answer"
            );
        }
        Ok(results.cooldown)
    }

    // active_cooldowns_from_results: correct every answered entry and keep the cooldowns that are still running
    pub fn active_from_results(
        &self,
        model_ids: &[String],
        results: &[Option<CooldownRecord>],
    ) -> Result<Vec<ActiveCooldown>, StoreError> {
        let now = (self.clock)();
        let mut active = Vec::with_capacity(results.len());
        for (model_id, result) in model_ids.iter().zip(results) {
            let Some(record) = result.clone() else {
                continue;
            };
            let key = RouteKey::parse(&format!("{KEY_PREFIX}{model_id}{KEY_SUFFIX}"))?;
            if let Some(cooldown) = self.corrected(&key, model_id, record, now) {
                active.push(cooldown);
            }
        }
        Ok(active)
    }

    // _corrected_active_cooldown: a past-deadline entry is dropped, and a promoted one gets the shorter of its real remaining time and a minute
    fn corrected(
        &self,
        key: &RouteKey,
        model_id: &str,
        record: CooldownRecord,
        now: f64,
    ) -> Option<ActiveCooldown> {
        let deadline = record.cooldown_time.mul_add(1.0_f64, record.timestamp);
        let remaining = (-now).mul_add(1.0_f64, deadline);
        let mut state = self
            .tier
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if remaining <= 0.0 {
            state.entries.remove(key);
            return None;
        }
        let longest = remaining.mul_add(1.0_f64, now).mul_add(1.0_f64, 5.0_f64);
        if state
            .entries
            .get(key)
            .is_some_and(|entry| entry.expires_at > longest)
        {
            let corrected = remaining.min(MAX_CORRECTED_IN_MEMORY_TTL_SECONDS);
            state.entries.insert(
                key.clone(),
                LocalEntry {
                    value: record.clone(),
                    expires_at: corrected.mul_add(1.0_f64, now),
                },
            );
        }
        Some(ActiveCooldown {
            model_id: model_id.to_owned(),
            remaining_seconds: remaining,
            record,
        })
    }

    // get_min_cooldown: the shortest configured cooldown time among the entries, else the router's default
    #[must_use]
    pub fn minimum_cooldown(&self, results: &[Option<CooldownRecord>]) -> f64 {
        let mut minimum: Option<f64> = None;
        for record in results.iter().flatten() {
            minimum = Some(minimum.map_or(record.cooldown_time, |found| {
                found.min(record.cooldown_time)
            }));
        }
        minimum.unwrap_or(self.default_cooldown_time)
    }
}
