use crate::trusted_network::cidr::CidrRanges;

// The typed general_settings keys the trusted-network functions read, resolved once by the config crate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrustedNetworkSettings {
    pub proxy_base_url: String,
    pub use_x_forwarded_for: bool,
    pub trusted_proxy_ranges: Option<CidrRanges>,
    pub mcp_trusted_proxy_ranges: Option<Vec<String>>,
    pub mcp_xff_num_trusted_hops: Option<i64>,
}

// network.TrustedProxyConfig: the ranges are strings that normalize_cidr_ranges has already trimmed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrustedProxyConfig {
    pub use_forwarded_for: bool,
    pub trusted_proxy_cidrs: Vec<String>,
}
