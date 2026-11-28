use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::Config;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::ClientSource;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_logging::logging_pre_post::env::{LogRecord, LogSink};

// one client per TLS policy and routing policy, the shapes a Python session can ask for
const CLIENT_POOL_CAPACITY: usize = 4;

// The httpx clients the SDK's handlers would pick per request, keyed by the policy each one was built with
pub(crate) struct ProxiedSource {
    config: Config,
    environment: BTreeMap<String, String>,
    routing: Arc<ProxyPolicy>,
    pool: ClientPool,
}

impl ProxiedSource {
    pub(crate) fn new(
        config: Config,
        environment: BTreeMap<String, String>,
        routing: Arc<ProxyPolicy>,
    ) -> Self {
        Self {
            config,
            environment,
            routing,
            pool: ClientPool::new(CLIENT_POOL_CAPACITY, TransportLimits::DEFAULT),
        }
    }
}

// The same client serves the create/list, read and write transports
impl ClientSource for ProxiedSource {
    type Client = ProxiedClient;

    fn client(&self, ssl_verify: Option<bool>) -> Result<ProxiedClient, ClientError> {
        let tls = TlsPolicy::resolve(&self.config, &self.environment, ssl_verify)
            .map_err(ClientError::from)?;
        self.pool
            .client(&tls, &self.routing)
            .map_err(ClientError::from)
    }
}

// The Python session configures no callback handlers, so the pre-call's records go nowhere
pub(crate) struct SilentSink;

impl LogSink for SilentSink {
    fn emit(&self, _record: LogRecord) {}
}
