#![cfg(test)]

use super::{PrometheusDailySpend, daily_spend_from_prometheus};
use crate::prometheus_query::error::QueryError;
use crate::prometheus_query::fake_transport::{FakeFailure, prometheus_query};
use rstest::rstest;
use rust_decimal::Decimal;
use std::str::FromStr as _;

const URL: Option<&str> = Some("http://prom:9090");

fn body(result: &str) -> Vec<u8> {
    format!(r#"{{"status":"success","data":{{"resultType":"matrix","result":[{result}]}}}}"#)
        .into_bytes()
}

#[tokio::test]
async fn queries_global_spend_over_thirty_days_with_a_one_day_step() {
    let query = prometheus_query(URL, None, vec![Ok(body(""))]);
    let spend = daily_spend_from_prometheus(&query, None).await.unwrap();
    assert!(spend.is_empty());
    let requests = query.transport.requests.lock().unwrap();
    assert_eq!(requests[0].url, "http://prom:9090/api/v1/query_range");
    assert_eq!(
        requests[0].params,
        [
            (
                "query",
                "sum(delta(litellm_spend_metric_total[1d]))".to_owned()
            ),
            ("start", "2024-07-19T16:53:20.250000+00:00".to_owned()),
            ("end", "2024-08-18T16:53:20.250000+00:00".to_owned()),
            ("step", "86400".to_owned()),
        ]
    );
}

#[tokio::test]
async fn queries_one_key_with_a_quoted_hashed_api_key_matcher() {
    let query = prometheus_query(URL, None, vec![Ok(body(""))]);
    daily_spend_from_prometheus(&query, Some("88dc28d0f0"))
        .await
        .unwrap();
    let requests = query.transport.requests.lock().unwrap();
    assert_eq!(
        requests[0].params[0].1,
        r#"sum(delta(litellm_spend_metric_total{hashed_api_key="88dc28d0f0"}[1d]))"#
    );
}

#[rstest]
#[case::label_breakout(r#"x"} or vector(1) or foo{a="b"#, r#"sum(delta(litellm_spend_metric_total{hashed_api_key="x\"} or vector(1) or foo{a=\"b"}[1d]))"#)]
#[case::trailing_backslash(
    r"abc\",
    r#"sum(delta(litellm_spend_metric_total{hashed_api_key="abc\\"}[1d]))"#
)]
#[case::newline(
    "a\nb",
    r#"sum(delta(litellm_spend_metric_total{hashed_api_key="a\nb"}[1d]))"#
)]
#[tokio::test]
async fn a_key_cannot_close_the_matcher_it_is_placed_in(#[case] key: &str, #[case] expected: &str) {
    let query = prometheus_query(URL, None, vec![Ok(body(""))]);
    daily_spend_from_prometheus(&query, Some(key))
        .await
        .unwrap();
    let requests = query.transport.requests.lock().unwrap();
    assert_eq!(requests[0].params[0].1, expected);
}

#[tokio::test]
async fn an_empty_key_filters_on_an_empty_hash_instead_of_global_spend() {
    let query = prometheus_query(URL, None, vec![Ok(body(""))]);
    daily_spend_from_prometheus(&query, Some("")).await.unwrap();
    let requests = query.transport.requests.lock().unwrap();
    assert_eq!(
        requests[0].params[0].1,
        r#"sum(delta(litellm_spend_metric_total{hashed_api_key=""}[1d]))"#
    );
}

#[tokio::test]
async fn returns_one_record_per_sample_in_series_and_sample_order() {
    let result = r#"{"metric":{},"values":[[1724025600,"1.5"],[1724112000,"2"]]},{"metric":{},"values":[[1723939200,"0.25"]]}"#;
    let query = prometheus_query(URL, None, vec![Ok(body(result))]);
    let spend = daily_spend_from_prometheus(&query, None).await.unwrap();
    let records: Vec<(&str, Decimal)> = spend
        .iter()
        .map(|record| (record.date.as_str(), record.spend))
        .collect();
    assert_eq!(
        records,
        [
            (
                "2024-08-19T00:00:00+00:00",
                Decimal::from_str("1.5").unwrap()
            ),
            ("2024-08-20T00:00:00+00:00", Decimal::from_str("2").unwrap()),
            (
                "2024-08-18T00:00:00+00:00",
                Decimal::from_str("0.25").unwrap()
            ),
        ]
    );
}

#[rstest]
#[case::whole_second("1724025600", "2024-08-19T00:00:00+00:00")]
#[case::half_second("1724025600.5", "2024-08-19T00:00:00.500000+00:00")]
#[case::microsecond("1724025600.000001", "2024-08-19T00:00:00.000001+00:00")]
#[case::rounds_to_microseconds("1724025600.1234567", "2024-08-19T00:00:00.123457+00:00")]
#[case::before_the_epoch("-1.5", "1969-12-31T23:59:58.500000+00:00")]
#[tokio::test]
async fn formats_the_sample_time_as_iso_8601_with_a_utc_offset(
    #[case] timestamp: &str,
    #[case] date: &str,
) {
    let result = format!(r#"{{"metric":{{}},"values":[[{timestamp},"1"]]}}"#);
    let query = prometheus_query(URL, None, vec![Ok(body(&result))]);
    let spend = daily_spend_from_prometheus(&query, None).await.unwrap();
    assert_eq!(spend[0].date, date);
}

#[tokio::test]
async fn reads_the_spend_value_exactly() {
    let result = r#"{"metric":{},"values":[[1724025600,"1.001818099998933"]]}"#;
    let query = prometheus_query(URL, None, vec![Ok(body(result))]);
    let spend = daily_spend_from_prometheus(&query, None).await.unwrap();
    assert_eq!(
        spend[0].spend,
        Decimal::from_str("1.001818099998933").unwrap()
    );
}

#[test]
fn serializes_a_record_with_a_json_number_for_the_spend() {
    let record = PrometheusDailySpend {
        date: "2024-08-18T00:00:00+00:00".to_owned(),
        spend: Decimal::from_str("1.001818099998933").unwrap(),
    };
    assert_eq!(
        serde_json::to_string(&record).unwrap(),
        r#"{"date":"2024-08-18T00:00:00+00:00","spend":1.001818099998933}"#
    );
}

#[tokio::test]
async fn fails_without_a_request_when_the_url_is_not_set() {
    let query = prometheus_query(None, None, vec![Ok(body(""))]);
    let error = daily_spend_from_prometheus(&query, None).await.unwrap_err();
    assert!(matches!(error, QueryError::NotConfigured));
    assert!(query.transport.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn rejects_a_spend_value_that_is_not_a_finite_number() {
    let result = r#"{"metric":{},"values":[[1724025600,"NaN"]]}"#;
    let query = prometheus_query(URL, None, vec![Ok(body(result))]);
    let error = daily_spend_from_prometheus(&query, None).await.unwrap_err();
    assert!(matches!(error, QueryError::SampleValue { value } if value == "NaN"));
}

#[tokio::test]
async fn rejects_a_timestamp_outside_the_date_range() {
    let result = r#"{"metric":{},"values":[[1e20,"1"]]}"#;
    let query = prometheus_query(URL, None, vec![Ok(body(result))]);
    let error = daily_spend_from_prometheus(&query, None).await.unwrap_err();
    assert!(matches!(error, QueryError::Timestamp { .. }), "{error}");
}

#[tokio::test]
async fn reports_a_transport_failure_with_its_cause() {
    let query = prometheus_query(URL, None, vec![Err(FakeFailure)]);
    let error = daily_spend_from_prometheus(&query, None).await.unwrap_err();
    assert!(matches!(error, QueryError::Transport(FakeFailure)));
}
