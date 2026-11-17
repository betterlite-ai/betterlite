use std::sync::Arc;

use rstest::rstest;

use super::support::{
    NoSettings, RESPONSE, build_in_offset, call_of, flush, kwargs, record, record_result, sums,
};

#[rstest]
#[case::end_from_the_call(r#"{"api_call_start_time": 10.5}"#, &["2.5"])]
#[case::end_from_the_kwargs(r#"{"api_call_start_time": 10.5, "end_time": 12}"#, &["1.5"])]
#[case::no_api_start("{}", &[])]
#[case::unparsed_api_start(r#"{"api_call_start_time": "soon"}"#, &[])]
#[case::unparsed_end(r#"{"api_call_start_time": 10.5, "end_time": "soon"}"#, &[])]
#[case::zero_length(r#"{"api_call_start_time": 13}"#, &[])]
#[case::negative_length(r#"{"api_call_start_time": 14}"#, &[])]
fn response_duration_runs_from_the_api_call_start_to_the_end(
    #[case] extra: &str,
    #[case] expected: &[&str],
) {
    let recorded = record(extra, None);
    assert_eq!(sums(&recorded, RESPONSE), expected);
}

#[test]
fn a_null_end_time_falls_back_to_the_call_end() {
    let recorded = record(r#"{"api_call_start_time": 10.5, "end_time": null}"#, None);
    assert_eq!(sums(&recorded, RESPONSE), ["2.5"]);
}

#[rstest]
#[case::with_microseconds("1970-01-01 01:00:10.500000", "2.5")]
#[case::without_microseconds("1970-01-01 01:00:10", "3")]
fn timestamp_text_is_read_in_the_configured_local_offset(
    #[case] text: &str,
    #[case] expected: &str,
) {
    let (recorder, provider, exporter) = build_in_offset(Arc::new(NoSettings), None, 3600);
    let call_kwargs = kwargs(&format!(r#"{{"api_call_start_time": "{text}"}}"#));
    recorder
        .record(&call_of(&call_kwargs, None))
        .expect("record");
    let recorded = flush(&provider, &exporter);
    assert_eq!(sums(&recorded, RESPONSE), [expected]);
}

#[test]
fn a_timestamp_of_the_wrong_type_is_an_error_naming_the_field() {
    let error =
        record_result(r#"{"api_call_start_time": {"seconds": 1}}"#, None).expect_err("wrong type");
    assert!(error.to_string().contains("api_call_start_time"));
}

#[test]
fn an_end_time_of_the_wrong_type_is_an_error_naming_the_field() {
    let error = record_result(r#"{"api_call_start_time": 10.5, "end_time": true}"#, None)
        .expect_err("wrong type");
    assert!(error.to_string().contains("end_time"));
}
