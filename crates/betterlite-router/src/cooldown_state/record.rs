use betterlite_redact::structured::SensitiveDataMasker;
use serde::{Deserialize, Serialize};

pub const KEY_PREFIX: &str = "deployment:";
pub const KEY_SUFFIX: &str = ":cooldown";
// the cap on the corrected in-memory TTL, so a shared entry deleted or extended early is noticed within a minute
pub const MAX_CORRECTED_TTL_SECONDS: f64 = 60.0;

// CooldownCacheValue: the entry one deployment's cooldown leaves in the shared cache
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CooldownRecord {
    pub exception_received: String,
    pub status_code: String,
    pub timestamp: f64,
    pub cooldown_time: f64,
}

impl CooldownRecord {
    // _common_add_cooldown_logic: the masked exception, the status as text, and the moment the cooldown started
    #[must_use]
    pub fn new(
        exception: &str,
        status_code: i64,
        timestamp: f64,
        cooldown_time: f64,
        masker: &SensitiveDataMasker,
    ) -> Self {
        Self {
            exception_received: masker.mask_value(exception),
            status_code: status_code.to_string(),
            timestamp,
            cooldown_time,
        }
    }
}
