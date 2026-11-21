use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_env::{EnvError, flag};

use crate::oauth_discovery::settings::DiscoverySettings as FetchSettings;

pub const ON_STARTUP_VARIABLE: &str = "LITELLM_MCP_OAUTH_DISCOVERY_ON_STARTUP";
const RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(50), Duration::from_millis(150)];
const BACKOFF_BASE: Duration = Duration::from_secs(30);
const BACKOFF_CAP: Duration = Duration::from_secs(900);
const TEMPORARY_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoverySettings {
    pub on_startup: bool,
    pub retry_delays: Vec<Duration>,
    pub backoff_base: Duration,
    pub backoff_cap: Duration,
    pub temporary_ttl: Duration,
    pub fetch: FetchSettings,
}

impl Default for DiscoverySettings {
    fn default() -> Self {
        Self {
            on_startup: false,
            retry_delays: RETRY_DELAYS.to_vec(),
            backoff_base: BACKOFF_BASE,
            backoff_cap: BACKOFF_CAP,
            temporary_ttl: TEMPORARY_TTL,
            fetch: FetchSettings::default(),
        }
    }
}

impl DiscoverySettings {
    pub fn from_lookup<E>(environment: &E) -> Result<Self, EnvError>
    where
        E: EnvironmentLookup,
    {
        let on_startup = environment
            .lookup(ON_STARTUP_VARIABLE)
            .map_or(Ok(None), |text| flag(ON_STARTUP_VARIABLE, &text))?
            .unwrap_or(false);
        Ok(Self {
            on_startup,
            retry_delays: RETRY_DELAYS.to_vec(),
            backoff_base: BACKOFF_BASE,
            backoff_cap: BACKOFF_CAP,
            temporary_ttl: TEMPORARY_TTL,
            fetch: FetchSettings::from_lookup(environment),
        })
    }

    #[must_use]
    pub fn backoff(&self, consecutive_failures: u32) -> Duration {
        let multiplier = 1_u32
            .checked_shl(consecutive_failures.saturating_sub(1))
            .unwrap_or(u32::MAX);
        self.backoff_base
            .checked_mul(multiplier)
            .map_or(self.backoff_cap, |delay| delay.min(self.backoff_cap))
    }
}
