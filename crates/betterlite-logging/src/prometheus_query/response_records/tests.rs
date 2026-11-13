#![cfg(test)]

use super::{PrometheusQueryResponse, PrometheusSample};
use rstest::rstest;
use rust_decimal::Decimal;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr as _;

#[test]
fn decodes_matrix_series_with_labels_and_timestamp_value_pairs() {
    let body = r#"{"status":"success","data":{"resultType":"matrix","result":[
        {"metric":{"primary_model":"gpt-4","fallback_model":"gpt-3.5"},"values":[[1435781430.781,"7"],[1435781445.781,"9"]]},
        {"metric":{},"values":[[1.5,"1"]]}]}}"#;
    let response: PrometheusQueryResponse = serde_json::from_str(body).unwrap();
    let result = response.data.result;
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].metric["primary_model"], "gpt-4");
    assert_eq!(result[0].metric["fallback_model"], "gpt-3.5");
    assert_eq!(
        result[0].values,
        vec![
            PrometheusSample {
                timestamp: Decimal::from_str("1435781430.781").unwrap(),
                value: "7".to_owned(),
            },
            PrometheusSample {
                timestamp: Decimal::from_str("1435781445.781").unwrap(),
                value: "9".to_owned(),
            },
        ]
    );
    assert!(result[1].metric.is_empty());
}

#[test]
fn keeps_the_order_of_the_result_list() {
    let body = r#"{"data":{"result":[
        {"metric":{"n":"b"},"values":[]},{"metric":{"n":"a"},"values":[]},{"metric":{"n":"c"},"values":[]}]}}"#;
    let response: PrometheusQueryResponse = serde_json::from_str(body).unwrap();
    let names: Vec<&str> = response
        .data
        .result
        .iter()
        .map(|series| series.metric["n"].as_str())
        .collect();
    assert_eq!(names, ["b", "a", "c"]);
}

#[test]
fn ignores_fields_the_records_do_not_declare() {
    let body = r#"{"status":"success","warnings":["x"],"data":{"resultType":"matrix","stats":{},"result":[{"metric":{},"values":[],"histograms":[]}]}}"#;
    let response: PrometheusQueryResponse = serde_json::from_str(body).unwrap();
    assert_eq!(response.data.result.len(), 1);
}

#[test]
fn decodes_an_empty_result_list() {
    let response: PrometheusQueryResponse =
        serde_json::from_str(r#"{"status":"success","data":{"result":[]}}"#).unwrap();
    assert!(response.data.result.is_empty());
}

#[rstest]
#[case::error_response(
    r#"{"status":"error","errorType":"bad_data","error":"parse error"}"#,
    "data"
)]
#[case::missing_result(r#"{"data":{"resultType":"matrix"}}"#, "result")]
#[case::missing_metric(r#"{"data":{"result":[{"values":[]}]}}"#, "metric")]
#[case::missing_values(r#"{"data":{"result":[{"metric":{}}]}}"#, "values")]
#[case::three_element_sample(r#"{"data":{"result":[{"metric":{},"values":[[1,"2",3]]}]}}"#, "")]
#[case::one_element_sample(r#"{"data":{"result":[{"metric":{},"values":[[1]]}]}}"#, "")]
#[case::numeric_value(r#"{"data":{"result":[{"metric":{},"values":[[1,2]]}]}}"#, "")]
#[case::non_string_label(r#"{"data":{"result":[{"metric":{"a":1},"values":[]}]}}"#, "")]
#[case::not_an_object("[]", "")]
fn rejects_responses_that_do_not_match_the_records(#[case] body: &str, #[case] field: &str) {
    let error = serde_json::from_str::<PrometheusQueryResponse>(body).unwrap_err();
    assert!(error.to_string().contains(field), "{error}");
}

#[rstest]
#[case::integer("7", Some("7"))]
#[case::fraction("2.5", Some("2.5"))]
#[case::exponent("1e+06", Some("1000000"))]
#[case::negative("-4", Some("-4"))]
#[case::not_a_number("NaN", None)]
#[case::infinity("+Inf", None)]
#[case::empty("", None)]
#[case::text("many", None)]
fn reads_a_sample_value_as_a_decimal(#[case] text: &str, #[case] expected: Option<&str>) {
    let sample = PrometheusSample {
        timestamp: Decimal::ZERO,
        value: text.to_owned(),
    };
    assert_eq!(
        sample.numeric_value(),
        expected.map(|digits| Decimal::from_str(digits).unwrap())
    );
}

#[rstest]
fn replays_the_prometheus_query_corpus(#[files("fuzz/corpus/prometheus_query/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(serde_json::from_slice::<PrometheusQueryResponse>(&bytes));
}
