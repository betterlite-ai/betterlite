use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, TlsPolicyError};
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::{ClientPool, PoolError};
use betterlite_provider::api::image_generation_http::ClientSource;

use super::super::error::SdkHttpError;

// The shared client each method selects when no supplied one matches, built with the ssl_verify the call carries
pub(super) struct Source {
    config: Config,
    environment: BTreeMap<String, String>,
    pool: ClientPool,
    routing: Arc<ProxyPolicy>,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum SourceError {
    #[error(transparent)]
    Tls(#[from] TlsPolicyError),
    #[error(transparent)]
    Pool(#[from] PoolError),
}

impl ClientSource for Source {
    type Client = ProxiedClient;
    type Error = SourceError;

    fn client(&self, ssl_verify: Option<bool>) -> Result<ProxiedClient, SourceError> {
        let tls = TlsPolicy::resolve(&self.config, &self.environment, ssl_verify)?;
        Ok(self.pool.client(&tls, &self.routing)?)
    }
}

pub(super) fn source(
    config: &Config,
    env: &BTreeMap<String, String>,
) -> Result<Source, SdkHttpError> {
    let routing = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    Ok(Source {
        config: config.clone(),
        environment: env.clone(),
        pool: ClientPool::new(1, TransportLimits::DEFAULT),
        routing,
    })
}
