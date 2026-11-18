use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use betterlite_config::loader::startup::process_environment;
use betterlite_config::ptu_pricing::load_ptu_cost_flag;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::{ExposeSecret as _, SecretString};

// is_ptu_cost_attribution_enabled, behind a port so a test answers without the process environment.
pub trait PtuAttribution: Send + Sync {
    fn enabled(&self) -> bool;
}

// The flag reads os.environ on every call, with the config's environment_variables section laid over it as startup exports that section into os.environ.
pub struct EnvironmentAttribution {
    pub configured: BTreeMap<String, Option<SecretString>>,
}

impl PtuAttribution for EnvironmentAttribution {
    fn enabled(&self) -> bool {
        let mut environment = process_environment();
        for (name, value) in &self.configured {
            let text = value
                .as_ref()
                .map_or_else(String::new, |secret| secret.expose_secret().to_owned());
            environment.insert(name.clone(), text);
        }
        let lookup = SecretLookup {
            manager: None,
            oidc_http: Arc::new(NoOidc),
            azure_tokens: Arc::new(NoOidc),
            clock: Arc::new(NoOidc),
            oidc_cache: OidcTokenCache::default(),
        };
        load_ptu_cost_flag(&lookup, &environment)
    }
}

// The flag name never carries the oidc/ prefix, so these ports are never reached; a secret manager joins the lookup with the startup that owns it.
pub(crate) struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the PTU flag reads no OIDC secrets".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        i64::try_from(elapsed).unwrap_or(i64::MAX)
    }
}
