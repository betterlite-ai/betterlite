use super::support::{Fixture, SentEnvelope, gunzip, header, record_at, record_count};
use crate::newrelic_metrics::flush::async_send_batch;
use crate::newrelic_metrics::queue::push_record;
use crate::newrelic_metrics::region::{EU_METRIC_ENDPOINT, US_METRIC_ENDPOINT};

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

#[tokio::test]
async fn gzip_headers_send_gzipped_json_to_the_metric_api() {
    let fixture = Fixture::new();
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    assert!(async_send_batch(&logger).await);
    let requests = fixture.transport.requests();
    let request = requests.first().expect("one Metric API request");
    assert_eq!(request.url, US_METRIC_ENDPOINT);
    assert_eq!(header(request, "content-type"), "application/json");
    assert_eq!(header(request, "content-encoding"), "gzip");
    assert!(request.body.starts_with(&GZIP_MAGIC));
    let envelopes: Vec<SentEnvelope> =
        serde_json::from_slice(&gunzip(&request.body)).expect("metric envelopes");
    assert_eq!(envelopes.len(), 1);
    assert_eq!(record_count(&envelopes[0]).to_bits(), 1.0_f64.to_bits());
}

#[tokio::test]
async fn gzip_headers_pass_the_api_key_unchanged() {
    let fixture = Fixture::new();
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    assert!(async_send_batch(&logger).await);
    let requests = fixture.transport.requests();
    let request = requests.first().expect("one Metric API request");
    assert_eq!(header(request, "api-key"), "team-key");
}

#[tokio::test]
async fn gzip_headers_use_the_eu_endpoint_for_the_eu_region() {
    let fixture = Fixture::new().with_region("EU");
    let logger = fixture.build();
    push_record(&logger, record_at(0));
    assert!(async_send_batch(&logger).await);
    let requests = fixture.transport.requests();
    let request = requests.first().expect("one Metric API request");
    assert_eq!(request.url, EU_METRIC_ENDPOINT);
}
