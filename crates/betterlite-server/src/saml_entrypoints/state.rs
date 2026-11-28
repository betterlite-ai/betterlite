use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::saml_provider::provider::SamlProvider;
use betterlite_auth::trusted_network::settings::TrustedNetworkSettings;

use crate::custom_url::CustomUrls;

#[derive(Clone)]
pub struct SamlEntrypointsState {
    pub provider: Arc<SamlProvider>,
    pub custom_urls: CustomUrls,
    pub trusted_network: TrustedNetworkSettings,
    pub environment: BTreeMap<String, String>,
}
