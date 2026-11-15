use std::num::NonZeroUsize;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use lru::LruCache;
use secrecy::SecretString;
use sha2::{Digest, Sha256};

pub const OBO_CACHE_MAX_ENTRIES: usize = 1000;
pub const DEFAULT_TOKEN_TTL_SECONDS: f64 = 3599.0;
pub const TOKEN_EXPIRY_SLACK_SECONDS: f64 = 60.0;

const OBO_CACHE_CAPACITY: NonZeroUsize = NonZeroUsize::new(OBO_CACHE_MAX_ENTRIES).unwrap();

#[derive(Debug)]
pub struct CachedOboToken {
    pub access_token: SecretString,
    // epoch seconds, as LiteLLM compares its float time.time()
    pub expires_at: f64,
}

#[derive(Debug)]
pub struct OboTokenCache {
    entries: LruCache<String, CachedOboToken>,
}

impl OboTokenCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: LruCache::new(OBO_CACHE_CAPACITY),
        }
    }

    #[must_use]
    pub fn lookup(&mut self, cache_key: &str, now: f64) -> Option<SecretString> {
        let cached = self.entries.peek(cache_key)?;
        // float_arithmetic is denied; a fused multiply-add adds the margin to the current time
        let threshold = TOKEN_EXPIRY_SLACK_SECONDS.mul_add(1.0_f64, now);
        if cached.expires_at > threshold {
            let token = cached.access_token.clone();
            let _promoted = self.entries.promote(cache_key);
            return Some(token);
        }
        None
    }

    // push evicts the least recently used entry once the cache is at capacity, as LiteLLM's popitem(last=False) does
    pub fn insert(&mut self, cache_key: String, token: CachedOboToken) {
        let _evicted = self.entries.push(cache_key, token);
    }

    pub fn evict(&mut self, cache_key: &str) {
        let _removed = self.entries.pop(cache_key);
    }

    #[must_use]
    pub fn expiry(&self, cache_key: &str) -> Option<f64> {
        self.entries.peek(cache_key).map(|token| token.expires_at)
    }

    #[must_use]
    pub fn snapshot(&self) -> Vec<(String, SecretString)> {
        self.entries
            .iter()
            .rev()
            .map(|(cache_key, token)| (cache_key.clone(), token.access_token.clone()))
            .collect()
    }
}

impl Default for OboTokenCache {
    fn default() -> Self {
        Self::new()
    }
}

#[must_use]
pub fn assertion_cache_key(assertion: &str) -> String {
    let digest = Sha256::digest(assertion.as_bytes());
    hex::encode(digest)
}

#[must_use]
pub fn clock_seconds(now: SystemTime) -> f64 {
    now.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[must_use]
pub fn parse_expires_in(raw: Option<&GuardrailValue>) -> f64 {
    match raw {
        Some(GuardrailValue::SignedInteger(seconds)) => parse_seconds(&seconds.to_string()),
        Some(GuardrailValue::UnsignedInteger(seconds)) => parse_seconds(&seconds.to_string()),
        Some(GuardrailValue::Decimal(seconds)) => *seconds,
        Some(GuardrailValue::Text(text)) => parse_seconds(text),
        Some(_) | None => DEFAULT_TOKEN_TTL_SECONDS,
    }
}

fn parse_seconds(text: &str) -> f64 {
    // Python's float() strips surrounding whitespace before parsing
    match text.trim().parse::<f64>() {
        Ok(seconds) => seconds,
        Err(_) => DEFAULT_TOKEN_TTL_SECONDS,
    }
}
