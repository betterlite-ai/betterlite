use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::{HeaderMap, Method};
use url::Url;

use crate::api::soniox::remote::transport::{RemoteTransport, Sleeper, TokioSleeper};

#[tokio::test]
async fn the_proxied_transport_sends_the_request_and_reports_the_refused_connection() {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("proxy policy"),
    );
    let client = ProxiedClient::outbound(&policy).expect("transport");
    let request = OutboundRequest {
        method: Method::GET,
        url: Url::parse("http://127.0.0.1:1/v1/transcriptions/job-1").expect("endpoint"),
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline: Duration::from_millis(500),
    };
    let error = RemoteTransport::call(&client, request)
        .await
        .expect_err("nothing listens on the closed endpoint");
    assert!(matches!(
        error,
        TransportError::Connect(_)
            | TransportError::ConnectTimeout(_)
            | TransportError::Deadline(_)
            | TransportError::Request(_)
    ));
}

#[tokio::test(start_paused = true)]
async fn the_tokio_sleeper_waits_the_requested_duration() {
    let start = tokio::time::Instant::now();
    TokioSleeper.sleep(Duration::from_secs(5)).await;
    assert_eq!(start.elapsed(), Duration::from_secs(5));
}
