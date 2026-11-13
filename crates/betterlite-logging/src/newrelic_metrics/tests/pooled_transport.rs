use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use http::HeaderMap;

use crate::newrelic_metrics::transport::{
    METRIC_API_TIMEOUT, NewRelicPost, NewRelicSendError, NewRelicTransport as _,
    PooledNewRelicTransport,
};

fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("an empty proxy policy"),
    );
    ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT)
        .expect("an outbound client")
}

#[tokio::test]
async fn pooled_transport_rejects_an_endpoint_that_is_not_a_url() {
    let transport = PooledNewRelicTransport {
        client: proxied_client(),
        deadline: METRIC_API_TIMEOUT,
    };
    let outcome = transport
        .post(NewRelicPost {
            url: "not a url",
            headers: HeaderMap::new(),
            body: Vec::new(),
        })
        .await;
    assert!(matches!(outcome, Err(NewRelicSendError::Url(_))));
}
