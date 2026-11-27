use std::sync::Arc;

use betterlite_auth::trusted_network::cidr::CidrRanges;
use betterlite_auth::trusted_network::settings::TrustedNetworkSettings;
use betterlite_config::types::Config;

use crate::state::shared::Shared;

// Pinned once at startup: the section already resolved every os.environ/ reference into typed values.
#[derive(Clone, Debug)]
pub struct Slot {
    pub settings: Arc<TrustedNetworkSettings>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        let section = &config.sections.trusted_network;
        Self {
            settings: Arc::new(TrustedNetworkSettings {
                proxy_base_url: section.proxy_base_url.clone(),
                use_x_forwarded_for: section.use_x_forwarded_for,
                trusted_proxy_ranges: section.trusted_proxy_ranges.clone().map(CidrRanges::List),
                mcp_trusted_proxy_ranges: section.mcp_trusted_proxy_ranges.clone(),
                mcp_xff_num_trusted_hops: section.mcp_xff_num_trusted_hops,
            }),
        }
    }
}
