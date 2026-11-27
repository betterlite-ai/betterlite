mod tests;

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use super::ConsentError;
use crate::byok_token_exchange::ports::{AuthCodeLookup, AuthCodeRecord};

// LiteLLM's _AUTH_CODE_TTL_SECONDS: a pending code is exchangeable for five minutes.
pub const AUTH_CODE_TTL_SECONDS: u32 = 300;
// LiteLLM's _AUTH_CODES_MAX_SIZE: a burst of abandoned flows cannot grow the store past this.
pub const AUTH_CODES_MAX_SIZE: usize = 1000;

#[derive(Debug, Default)]
pub struct AuthCodeStore {
    codes: Mutex<HashMap<String, AuthCodeRecord>>,
}

impl AuthCodeStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            codes: Mutex::new(HashMap::new()),
        }
    }

    // LiteLLM refuses a new flow once 1000 are pending, before it generates a code.
    pub fn admit(&self) -> Result<(), ConsentError> {
        let codes = self.codes.lock().unwrap_or_else(PoisonError::into_inner);
        if codes.len() >= AUTH_CODES_MAX_SIZE {
            return Err(ConsentError::TooManyPendingFlows);
        }
        Ok(())
    }

    // A UUID4 collision would replace a live flow, so the replacement is logged instead of dropped.
    pub fn insert(&self, code: String, record: AuthCodeRecord) {
        let mut codes = self.codes.lock().unwrap_or_else(PoisonError::into_inner);
        if codes.insert(code, record).is_some() {
            tracing::warn!(
                "mcp byok consent: a generated authorization code replaced a pending flow"
            );
        }
    }

    #[must_use]
    pub fn count(&self) -> usize {
        let codes = self.codes.lock().unwrap_or_else(PoisonError::into_inner);
        codes.len()
    }
}

impl AuthCodeLookup for AuthCodeStore {
    // LiteLLM's _purge_expired_codes drops a code once expires_at < now.
    fn purge_expired(&self, now: f64) {
        let mut codes = self.codes.lock().unwrap_or_else(PoisonError::into_inner);
        codes.retain(|_code, record| record.expires_at >= now);
    }

    fn get(&self, code: &str) -> Option<AuthCodeRecord> {
        let codes = self.codes.lock().unwrap_or_else(PoisonError::into_inner);
        codes.get(code).cloned()
    }

    fn remove(&self, code: &str) {
        let mut codes = self.codes.lock().unwrap_or_else(PoisonError::into_inner);
        codes.remove(code);
    }
}
