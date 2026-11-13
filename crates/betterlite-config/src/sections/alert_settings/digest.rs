use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_integer, resolve_number};
use crate::sections::alert_settings::flags::{RawFlag, resolve_flag};
use crate::sections::alert_settings::types::{AlertType, AlertTypeConfig, DEFAULT_DIGEST_INTERVAL};

const CONFIG_PATH: &str = "general_settings.alert_type_config";

#[derive(Debug, Default, Deserialize)]
pub struct RawAlertTypeConfig {
    digest: Option<RawFlag>,
    digest_interval: Option<RawNumber>,
}

impl RawAlertTypeConfig {
    pub fn resolve<L>(
        self,
        alert_type: AlertType,
        lookup: &L,
    ) -> Result<AlertTypeConfig, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let path = format!("{CONFIG_PATH}.{alert_type}");
        let digest = self.digest.map_or(Ok(false), |raw| {
            resolve_flag(raw, &format!("{path}.digest"), lookup)
        })?;
        let digest_interval = resolve_number(
            self.digest_interval,
            &format!("{path}.digest_interval"),
            lookup,
            parse_integer,
        )?
        .unwrap_or(DEFAULT_DIGEST_INTERVAL);
        Ok(AlertTypeConfig {
            digest,
            digest_interval,
        })
    }
}
