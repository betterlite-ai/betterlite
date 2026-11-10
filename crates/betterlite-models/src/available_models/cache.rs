use std::collections::BTreeMap;
use std::sync::Arc;

use secrecy::ExposeSecret;
use serde::Serialize;
use sha1::{Digest as _, Sha1};

use crate::available_models::environment::EnvironmentSnapshot;
use crate::available_models::parameters::LiteLlmParams;
use crate::identity::provider_id::ProviderId;

pub const CACHE_CAPACITY: usize = 300;
pub const CACHE_TTL_SECONDS: f64 = 1000.0;

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("the model cache clock is not a finite number")]
    InvalidClock,
    #[error("the model cache holds values without an expiry to evict")]
    MissingExpiry,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CacheLimits {
    pub capacity: usize,
    pub default_ttl: f64,
}

pub struct AvailableModelsCache {
    limits: CacheLimits,
    values: BTreeMap<String, Vec<String>>,
    expiries: BTreeMap<String, f64>,
    environment_fingerprint: Option<u64>,
    clock: Arc<dyn Fn() -> f64 + Send + Sync>,
}

// The key never leaves the process, so the secret text is exposed only to the hash below.
#[derive(Serialize)]
struct ParamsFingerprint<'params> {
    model: &'params str,
    api_key: Option<&'params str>,
    api_base: Option<&'params str>,
    api_version: Option<&'params str>,
    custom_llm_provider: Option<ProviderId>,
}

impl<'params> From<&'params LiteLlmParams> for ParamsFingerprint<'params> {
    fn from(parameters: &'params LiteLlmParams) -> Self {
        Self {
            model: &parameters.model,
            api_key: parameters.api_key.as_ref().map(ExposeSecret::expose_secret),
            api_base: parameters.api_base.as_deref(),
            api_version: parameters.api_version.as_deref(),
            custom_llm_provider: parameters.custom_llm_provider,
        }
    }
}

impl AvailableModelsCache {
    pub fn with_clock(clock: Arc<dyn Fn() -> f64 + Send + Sync>) -> Self {
        Self {
            limits: CacheLimits {
                capacity: CACHE_CAPACITY,
                default_ttl: CACHE_TTL_SECONDS,
            },
            values: BTreeMap::new(),
            expiries: BTreeMap::new(),
            environment_fingerprint: None,
            clock,
        }
    }

    pub fn get(
        &mut self,
        provider: Option<ProviderId>,
        parameters: Option<&LiteLlmParams>,
        environment: &EnvironmentSnapshot,
    ) -> Result<Option<Vec<String>>, CacheError> {
        if parameters.is_none() {
            let changed = self
                .environment_fingerprint
                .is_none_or(|stored| stored != environment.fingerprint);
            self.environment_fingerprint
                .get_or_insert(environment.fingerprint);
            if changed {
                self.values.clear();
                return Ok(None);
            }
        }
        let key = cache_key(provider, parameters);
        if !self.values.contains_key(&key) {
            return Ok(None);
        }
        let now = self.read_clock()?;
        if self.expiries.get(&key).is_some_and(|expiry| now > *expiry) {
            self.values.remove(&key);
            self.expiries.remove(&key);
            return Ok(None);
        }
        Ok(self.values.get(&key).cloned())
    }

    pub fn set(
        &mut self,
        provider: ProviderId,
        parameters: Option<&LiteLlmParams>,
        models: &[String],
    ) -> Result<(), CacheError> {
        let now = self.read_clock()?;
        self.prune_expired(now);
        while self.values.len() >= self.limits.capacity {
            self.evict_earliest()?;
        }
        let key = cache_key(Some(provider), parameters);
        let refreshed = self.expiries.get(&key).is_none_or(|expiry| *expiry < now);
        self.values.insert(key.clone(), models.to_vec());
        if refreshed {
            let expiry = now.mul_add(1.0_f64, self.limits.default_ttl);
            self.expiries.insert(key, expiry);
        }
        Ok(())
    }

    // every stored absolute expiry, including keys an environment clear dropped the value of
    #[must_use]
    pub fn ttl_entries(&self) -> Vec<(String, f64)> {
        self.expiries
            .iter()
            .map(|(key, expiry)| (key.clone(), *expiry))
            .collect()
    }

    fn read_clock(&self) -> Result<f64, CacheError> {
        let now = (self.clock)();
        if now.is_finite() {
            Ok(now)
        } else {
            Err(CacheError::InvalidClock)
        }
    }

    fn prune_expired(&mut self, now: f64) {
        let expired: Vec<String> = self
            .expiries
            .iter()
            .filter(|(_key, expiry)| **expiry <= now)
            .map(|(key, _expiry)| key.clone())
            .collect();
        for key in expired {
            self.values.remove(&key);
            self.expiries.remove(&key);
        }
    }

    fn evict_earliest(&mut self) -> Result<(), CacheError> {
        let earliest = self
            .expiries
            .iter()
            .min_by(|left, right| left.1.total_cmp(right.1).then_with(|| left.0.cmp(right.0)))
            .map(|(key, _expiry)| key.clone())
            .ok_or(CacheError::MissingExpiry)?;
        self.values.remove(&earliest);
        self.expiries.remove(&earliest);
        Ok(())
    }
}

fn cache_key(provider: Option<ProviderId>, parameters: Option<&LiteLlmParams>) -> String {
    let serialized = parameters.map_or_else(String::new, |params| {
        // Every field serializes without failing: no map keys, no floats, no arbitrary types.
        serde_json::to_string(&ParamsFingerprint::from(params)).unwrap_or_default()
    });
    let prefixed = match provider {
        Some(named) => format!("{named}:{serialized}"),
        None => serialized,
    };
    let mut hasher = Sha1::new();
    hasher.update(prefixed.as_bytes());
    hex::encode(hasher.finalize())
}

impl AsRef<CacheLimits> for AvailableModelsCache {
    fn as_ref(&self) -> &CacheLimits {
        &self.limits
    }
}
