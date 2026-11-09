use serde::Deserialize;

use crate::sections::alert_settings::types::{AlertTypeConfig, DEFAULT_DIGEST_INTERVAL};

#[derive(Debug, Default, Deserialize)]
pub struct RawAlertTypeConfig {
    digest: Option<bool>,
    digest_interval: Option<u32>,
}

impl RawAlertTypeConfig {
    #[must_use]
    pub fn resolve(self) -> AlertTypeConfig {
        AlertTypeConfig {
            digest: self.digest.unwrap_or(false),
            digest_interval: self.digest_interval.unwrap_or(DEFAULT_DIGEST_INTERVAL),
        }
    }
}
