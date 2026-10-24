use std::sync::Arc;

use betterlite_config::proxy::{ProxyPolicy, ProxyRoute};
use reqwest::{Client, ClientBuilder, Proxy, Request, Response};
use secrecy::ExposeSecret as _;

use crate::error::ProxyError;

#[derive(Debug)]
pub struct ProxiedClient {
    pub client: Client,
    pub policy: Arc<ProxyPolicy>,
}

impl ProxiedClient {
    pub fn build(builder: ClientBuilder, policy: &Arc<ProxyPolicy>) -> Result<Self, ProxyError> {
        for warning in &policy.warnings {
            tracing::warn!(?warning, "proxy setting ignored");
        }
        // reqwest would otherwise read the process environment on its own
        let mut configured = builder.no_proxy();
        for route in policy.primary.values().chain(policy.fallback.values()) {
            let ProxyRoute::Forward(server) = route else {
                continue;
            };
            let target = format!("http://{}:{}", server.host, server.port);
            let routing = Arc::clone(policy);
            let chosen = Arc::clone(server);
            let mut proxy = Proxy::custom(move |url| match routing.route(url) {
                Some(ProxyRoute::Forward(found)) if Arc::ptr_eq(found, &chosen) => {
                    Some(target.clone())
                }
                Some(
                    ProxyRoute::Forward(_)
                    | ProxyRoute::Unsupported { .. }
                    | ProxyRoute::Invalid { .. },
                )
                | None => None,
            });
            if let Some(credentials) = &server.credentials {
                let password = credentials.password.expose_secret();
                proxy = proxy.basic_auth(&credentials.username, password);
            }
            configured = configured.proxy(proxy);
        }
        let client = configured
            .build()
            .map_err(|source| ProxyError::Client { source })?;
        Ok(Self {
            client,
            policy: Arc::clone(policy),
        })
    }

    pub async fn execute(&self, request: Request) -> Result<Response, ProxyError> {
        match self.policy.route(request.url()) {
            Some(ProxyRoute::Unsupported { scheme }) => {
                return Err(ProxyError::UnsupportedScheme {
                    scheme: scheme.clone(),
                });
            }
            Some(ProxyRoute::Invalid { name }) => {
                return Err(ProxyError::InvalidUrl { name: name.clone() });
            }
            Some(ProxyRoute::Forward(_)) | None => {}
        }
        self.client
            .execute(request)
            .await
            .map_err(|source| ProxyError::Request { source })
    }
}

mod tests;
