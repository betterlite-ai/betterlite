use std::sync::Arc;

use betterlite_config::proxy::{ProxyPolicy, ProxyRoute, ProxyServer};
use reqwest::Proxy;
use secrecy::ExposeSecret as _;

#[derive(Clone, Copy)]
pub(super) enum Follow {
    Redirects,
    Never,
}

pub(super) fn forward_proxy(policy: &Arc<ProxyPolicy>, server: &Arc<ProxyServer>) -> Proxy {
    let target = format!("http://{}:{}", server.host, server.port);
    let routing = Arc::clone(policy);
    let chosen = Arc::clone(server);
    let proxy = Proxy::custom(move |url| match routing.route(url) {
        Some(ProxyRoute::Forward(found)) if Arc::ptr_eq(found, &chosen) => Some(target.clone()),
        Some(
            ProxyRoute::Forward(_) | ProxyRoute::Unsupported { .. } | ProxyRoute::Invalid { .. },
        )
        | None => None,
    });
    match &server.credentials {
        Some(credentials) => {
            proxy.basic_auth(&credentials.username, credentials.password.expose_secret())
        }
        None => proxy,
    }
}
