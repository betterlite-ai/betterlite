pub mod error;

use std::sync::Arc;

use betterlite_auth::auth_admission::ports::{DestinationSeedError, TenantDestinations};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_core::api::request_setup_options::{CallbackVarsDecryption, RequestSetupOptions};
use betterlite_logging::otel::fan_out::attach::deliverable_destinations;
use betterlite_logging::otel::logger_runtime::LoggerRuntime;
use betterlite_logging::otel::logger_runtime::publish::fan_out_provider;
use betterlite_logging::otel::proxy_runtime::slot::set_destinations;
use indexmap::IndexMap;

use self::error::TenantDestinationError;

// _seed_request_destinations: resolve, then the published fan-out's buildability filter, then the request slot
pub struct TenantDestinationAdapter<Ids, Decryption, Environment> {
    pub setup: RequestSetupOptions<Ids, Decryption, Environment>,
    pub runtime: Arc<LoggerRuntime>,
}

impl<Ids, Decryption, Environment> TenantDestinations
    for TenantDestinationAdapter<Ids, Decryption, Environment>
where
    Ids: Send + Sync,
    Decryption: CallbackVarsDecryption + Send + Sync,
    Environment: EnvironmentLookup + Send + Sync,
{
    fn seed(
        &self,
        auth: &UserAPIKeyAuth,
        headers: &IndexMap<String, String>,
    ) -> Result<(), DestinationSeedError> {
        // the port hands over the auth object; the resolver reads the same fields off the key's serde form
        let principal: PrincipalFacts = serde_json::to_value(auth)
            .and_then(serde_json::from_value)
            .map_err(TenantDestinationError::Principal)?;
        let resolved = self.setup.tenant_destinations(&principal, headers);
        // the provider is read per request, so a fan-out the logger publishes later still carries the destinations
        let deliverable = {
            let provider = fan_out_provider(&self.runtime);
            let registry = provider.registry();
            deliverable_destinations(&resolved.destinations, &registry)
        };
        // an empty success replaces an earlier list; the filter runs first, so an unbuildable destination never suppresses the operator
        set_destinations(deliverable);
        Ok(())
    }
}

impl From<TenantDestinationError> for DestinationSeedError {
    fn from(source: TenantDestinationError) -> Self {
        Self {
            source: Box::new(source),
        }
    }
}

mod tests;
