#![cfg(test)]

use super::fallback_from_prometheus;
use crate::prometheus_query::error::QueryError;
use crate::prometheus_query::fake_transport::{FakeFailure, prometheus_query};
use rstest::rstest;

const URL: Option<&str> = Some("http://prom:9090");
const EMPTY: &str = r#"{"data":{"result":[]}}"#;

fn body(result: &str) -> Vec<u8> {
    format!(r#"{{"data":{{"result":[{result}]}}}}"#).into_bytes()
}

#[tokio::test]
async fn queries_the_successful_then_the_failed_fallback_totals() {
    let query = prometheus_query(
        URL,
        None,
        vec![Ok(EMPTY.as_bytes().to_vec()), Ok(EMPTY.as_bytes().to_vec())],
    );
    let message = fallback_from_prometheus(&query).await.unwrap();
    assert_eq!(message, "");
    let queries: Vec<String> = query
        .transport
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.params[0].1.clone())
        .collect();
    assert_eq!(
        queries,
        [
            "litellm_deployment_successful_fallbacks_total[24h]",
            "litellm_deployment_failed_fallbacks_total[24h]",
        ]
    );
}

#[tokio::test]
async fn words_the_failed_fallback_series_as_successful_requests() {
    let successful = body(
        r#"{"metric":{"primary_model":"gpt-4","fallback_model":"claude"},"values":[[1,"3"]]}"#,
    );
    let failed =
        body(r#"{"metric":{"primary_model":"gpt-4","fallback_model":"llama"},"values":[[1,"5"]]}"#);
    let query = prometheus_query(URL, None, vec![Ok(successful), Ok(failed)]);
    let message = fallback_from_prometheus(&query).await.unwrap();
    assert_eq!(
        message,
        "`3 successful fallback requests` with primary model=`gpt-4` -> fallback model=`claude`\n\
         `5 successful fallback requests` with primary model=`gpt-4` -> fallback model=`llama`\n"
    );
}

#[tokio::test]
async fn keeps_the_result_order_within_and_across_metrics() {
    let successful = body(
        r#"{"metric":{"primary_model":"z"},"values":[[1,"1"]]},{"metric":{"primary_model":"a"},"values":[[1,"2"]]}"#,
    );
    let failed = body(r#"{"metric":{"primary_model":"m"},"values":[[1,"3"]]}"#);
    let query = prometheus_query(URL, None, vec![Ok(successful), Ok(failed)]);
    let message = fallback_from_prometheus(&query).await.unwrap();
    let models: Vec<&str> = message
        .lines()
        .map(|line| {
            line.split("primary model=`")
                .nth(1)
                .unwrap()
                .split('`')
                .next()
                .unwrap()
        })
        .collect();
    assert_eq!(models, ["z", "a", "m"]);
}

#[tokio::test]
async fn reads_only_the_first_sample_of_each_series() {
    let successful = body(r#"{"metric":{},"values":[[1,"3"],[2,"9"]]}"#);
    let query = prometheus_query(
        URL,
        None,
        vec![Ok(successful), Ok(EMPTY.as_bytes().to_vec())],
    );
    let message = fallback_from_prometheus(&query).await.unwrap();
    assert_eq!(
        message,
        "`3 successful fallback requests` with primary model=`Unknown` -> fallback model=`Unknown`\n"
    );
}

#[rstest]
#[case::integer("12", "12")]
#[case::fraction_truncates_down("2.9", "2")]
#[case::negative_fraction_is_zero("-0.4", "0")]
#[case::negative("-3.7", "-3")]
#[case::exponent("1e+03", "1000")]
#[tokio::test]
async fn shows_the_first_value_as_a_whole_number(#[case] sample: &str, #[case] shown: &str) {
    let successful = body(&format!(r#"{{"metric":{{}},"values":[[1,"{sample}"]]}}"#));
    let query = prometheus_query(
        URL,
        None,
        vec![Ok(successful), Ok(EMPTY.as_bytes().to_vec())],
    );
    let message = fallback_from_prometheus(&query).await.unwrap();
    assert!(
        message.starts_with(&format!("`{shown} successful")),
        "{message}"
    );
}

#[rstest]
#[case::matching_instance(Some("web-1:8000"), "web-1:8000", true)]
#[case::other_instance(Some("web-1:8000"), "web-2:8000", false)]
#[case::no_filter(None, "web-2:8000", true)]
#[case::empty_filter_counts_as_unset(Some(""), "web-2:8000", true)]
#[tokio::test]
async fn keeps_only_the_selected_instance(
    #[case] selected: Option<&str>,
    #[case] instance: &str,
    #[case] kept: bool,
) {
    let successful = body(&format!(
        r#"{{"metric":{{"instance":"{instance}"}},"values":[[1,"3"]]}}"#
    ));
    let query = prometheus_query(
        URL,
        selected,
        vec![Ok(successful), Ok(EMPTY.as_bytes().to_vec())],
    );
    let message = fallback_from_prometheus(&query).await.unwrap();
    assert_eq!(!message.is_empty(), kept);
}

#[tokio::test]
async fn skips_a_series_without_an_instance_label_when_an_instance_is_selected() {
    let successful = body(r#"{"metric":{"primary_model":"a"},"values":[[1,"3"]]}"#);
    let query = prometheus_query(
        URL,
        Some("web-1:8000"),
        vec![Ok(successful), Ok(EMPTY.as_bytes().to_vec())],
    );
    assert_eq!(fallback_from_prometheus(&query).await.unwrap(), "");
}

#[tokio::test]
async fn skips_a_series_without_samples_instead_of_failing() {
    let successful = body(
        r#"{"metric":{"primary_model":"empty"},"values":[]},{"metric":{"primary_model":"b"},"values":[[1,"2"]]}"#,
    );
    let query = prometheus_query(
        URL,
        None,
        vec![Ok(successful), Ok(EMPTY.as_bytes().to_vec())],
    );
    let message = fallback_from_prometheus(&query).await.unwrap();
    assert_eq!(message.lines().count(), 1);
    assert!(message.contains("primary model=`b`"));
}

#[tokio::test]
async fn ignores_the_value_of_a_series_the_instance_filter_drops() {
    let successful = body(r#"{"metric":{"instance":"other"},"values":[[1,"NaN"]]}"#);
    let query = prometheus_query(
        URL,
        Some("web-1:8000"),
        vec![Ok(successful), Ok(EMPTY.as_bytes().to_vec())],
    );
    assert_eq!(fallback_from_prometheus(&query).await.unwrap(), "");
}

#[rstest]
#[case::not_a_number("NaN")]
#[case::infinity("+Inf")]
#[tokio::test]
async fn rejects_a_value_that_is_not_a_finite_number(#[case] sample: &str) {
    let successful = body(&format!(r#"{{"metric":{{}},"values":[[1,"{sample}"]]}}"#));
    let query = prometheus_query(URL, None, vec![Ok(successful)]);
    let error = fallback_from_prometheus(&query).await.unwrap_err();
    assert!(matches!(error, QueryError::SampleValue { value } if value == sample));
}

#[tokio::test]
async fn fails_when_the_url_is_not_set() {
    let query = prometheus_query(None, None, Vec::new());
    let error = fallback_from_prometheus(&query).await.unwrap_err();
    assert!(matches!(error, QueryError::NotConfigured));
}

#[tokio::test]
async fn a_failed_second_query_fails_the_whole_summary() {
    let successful = body(r#"{"metric":{},"values":[[1,"3"]]}"#);
    let query = prometheus_query(URL, None, vec![Ok(successful), Err(FakeFailure)]);
    let error = fallback_from_prometheus(&query).await.unwrap_err();
    assert!(matches!(error, QueryError::Transport(FakeFailure)));
}
