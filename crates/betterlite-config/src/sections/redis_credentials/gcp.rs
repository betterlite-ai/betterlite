use secrecy::SecretString;

use crate::loader::EnvironmentLookup;

use super::{RawSettings, ssl_enabled};

const GCP_TOKEN_TTL_SECONDS: u64 = 3300;

#[derive(Debug, Clone)]
pub struct GcpSettings {
    pub service_account: SecretString,
    pub ssl_ca_certs: Option<String>,
    pub token_ttl_seconds: u64,
}

pub(super) fn build_gcp<L>(settings: &RawSettings, lookup: &L) -> Option<GcpSettings>
where
    L: EnvironmentLookup,
{
    let gcp_service_account = settings
        .gcp_service_account
        .clone()
        .filter(|value| !value.is_empty())
        .or_else(|| lookup.lookup("REDIS_GCP_SERVICE_ACCOUNT"));
    let gcp_ssl_ca_certs = settings
        .gcp_ssl_ca_certs
        .clone()
        .or_else(|| lookup.lookup("REDIS_GCP_SSL_CA_CERTS"));
    let ssl = ssl_enabled(settings, lookup);
    gcp_service_account.map(|account| GcpSettings {
        service_account: SecretString::from(account),
        ssl_ca_certs: gcp_ssl_ca_certs.filter(|_| ssl),
        token_ttl_seconds: GCP_TOKEN_TTL_SECONDS,
    })
}
