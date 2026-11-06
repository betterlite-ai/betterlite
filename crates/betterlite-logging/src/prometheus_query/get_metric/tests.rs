#![cfg(test)]

use super::get_metric_from_prometheus;
use crate::prometheus_query::error::QueryError;
use crate::prometheus_query::fake_transport::{FakeFailure, prometheus_query};
use crate::prometheus_query::transport::{MAX_RESPONSE_BYTES, QUERY_TIMEOUT};
use rstest::rstest;

const ONE_SERIES: &str = r#"{"status":"success","data":{"resultType":"matrix","result":[{"metric":{"model":"a"},"values":[[1724000000,"4"]]},{"metric":{"model":"b"},"values":[[1724000000,"5"]]}]}}"#;

#[tokio::test]
async fn queries_the_metric_over_a_24_hour_range_selector_at_the_current_second() {
    let query = prometheus_query(
        Some("http://prom:9090"),
        None,
        vec![Ok(ONE_SERIES.as_bytes().to_vec())],
    );
    get_metric_from_prometheus(&query, "litellm_requests_metric_total")
        .await
        .unwrap();
    let requests = query.transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "http://prom:9090/api/v1/query");
    assert_eq!(
        requests[0].params,
        [
            ("query", "litellm_requests_metric_total[24h]".to_owned()),
            ("time", "1724000000".to_owned()),
        ]
    );
}

#[tokio::test]
async fn returns_the_result_list_in_response_order() {
    let query = prometheus_query(
        Some("http://prom:9090"),
        None,
        vec![Ok(ONE_SERIES.as_bytes().to_vec())],
    );
    let result = get_metric_from_prometheus(&query, "m").await.unwrap();
    let models: Vec<&str> = result
        .iter()
        .map(|series| series.metric["model"].as_str())
        .collect();
    assert_eq!(models, ["a", "b"]);
    assert_eq!(result[1].values[0].value, "5");
}

#[tokio::test]
async fn bounds_the_request_by_time_and_response_size() {
    let query = prometheus_query(
        Some("http://prom:9090"),
        None,
        vec![Ok(ONE_SERIES.as_bytes().to_vec())],
    );
    get_metric_from_prometheus(&query, "m").await.unwrap();
    let requests = query.transport.requests.lock().unwrap();
    assert_eq!(requests[0].timeout, QUERY_TIMEOUT);
    assert_eq!(requests[0].max_response_bytes, MAX_RESPONSE_BYTES);
}

#[tokio::test]
async fn fails_without_a_request_when_the_url_is_not_set() {
    let query = prometheus_query(None, None, vec![Ok(ONE_SERIES.as_bytes().to_vec())]);
    let error = get_metric_from_prometheus(&query, "m").await.unwrap_err();
    assert!(matches!(error, QueryError::NotConfigured));
    assert!(query.transport.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reports_a_transport_failure_with_its_cause() {
    let query = prometheus_query(Some("http://prom:9090"), None, vec![Err(FakeFailure)]);
    let error = get_metric_from_prometheus(&query, "m").await.unwrap_err();
    assert!(matches!(error, QueryError::Transport(FakeFailure)));
}

#[rstest]
#[case::error_body(br#"{"status":"error","error":"bad query"}"#.to_vec())]
#[case::not_json(b"<html>bad gateway</html>".to_vec())]
#[case::empty(Vec::new())]
#[tokio::test]
async fn rejects_a_body_that_is_not_a_query_response(#[case] body: Vec<u8>) {
    let query = prometheus_query(Some("http://prom:9090"), None, vec![Ok(body)]);
    let error = get_metric_from_prometheus(&query, "m").await.unwrap_err();
    assert!(matches!(error, QueryError::Decode(_)), "{error}");
}

#[tokio::test]
async fn rejects_a_body_over_the_size_limit() {
    let body = vec![b' '; MAX_RESPONSE_BYTES + 1];
    let query = prometheus_query(Some("http://prom:9090"), None, vec![Ok(body)]);
    let error = get_metric_from_prometheus(&query, "m").await.unwrap_err();
    assert!(
        matches!(error, QueryError::ResponseTooLarge { length, limit } if length == MAX_RESPONSE_BYTES + 1 && limit == MAX_RESPONSE_BYTES)
    );
}

#[tokio::test]
async fn accepts_a_body_of_exactly_the_size_limit() {
    let mut body = ONE_SERIES.as_bytes().to_vec();
    body.resize(MAX_RESPONSE_BYTES, b' ');
    let query = prometheus_query(Some("http://prom:9090"), None, vec![Ok(body)]);
    assert_eq!(
        get_metric_from_prometheus(&query, "m").await.unwrap().len(),
        2
    );
}
