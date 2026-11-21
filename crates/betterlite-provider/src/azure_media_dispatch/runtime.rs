use std::time::Duration;

use betterlite_azure::credentials::source::CredentialSources;
use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_config::sections::azure_url_policy::AzureCredentials;

// The resolved Azure settings and the injected leaves every arm of one dispatch reads
pub struct MediaRuntime<'call, C, K, T, L> {
    pub endpoints: &'call EndpointSettings,
    pub credentials: &'call AzureCredentials,
    pub environment: &'call dyn EnvironmentLookup,
    pub sources: &'call dyn CredentialSources,
    pub refresh_enabled: bool,
    pub polling_timeout: Duration,
    pub constructor: &'call C,
    pub clock: &'call K,
    pub transport: &'call T,
    pub cache: &'call mut dyn ClientCache,
    pub logger: &'call mut L,
}
