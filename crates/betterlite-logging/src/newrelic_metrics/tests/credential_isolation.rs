use temp_env::with_vars;

use super::support::{Fixture, header, record_at};
use crate::newrelic_metrics::NewRelicMetricsError;
use crate::newrelic_metrics::flush::async_send_batch;
use crate::newrelic_metrics::queue::push_record;

fn environment() -> [(&'static str, Option<&'static str>); 2] {
    [
        ("NEWRELIC_API_KEY", Some("environment-key")),
        ("NEW_RELIC_API_KEY", Some("environment-key")),
    ]
}

#[tokio::test]
async fn credential_isolation_never_falls_back_to_the_environment() {
    let fixture = Fixture::new();
    let outcome = with_vars(environment(), || fixture.with_key("").build_result());
    let Err(error) = outcome else {
        panic!("an empty team ingest key still fails");
    };
    assert!(matches!(error, NewRelicMetricsError::MissingApiKey));
}

#[tokio::test]
async fn credential_isolation_sends_only_the_constructor_key() {
    let mut fixture = Fixture::new();
    fixture.api_key = "constructor-key".to_owned();
    let logger = with_vars(environment(), || fixture.build());
    push_record(&logger, record_at(0));
    assert!(async_send_batch(&logger).await);
    let requests = fixture.transport.requests();
    let request = requests.first().expect("one Metric API request");
    assert_eq!(header(request, "api-key"), "constructor-key");
}
