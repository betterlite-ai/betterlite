#![cfg(test)]

use super::{GatewayCacheAttribution, RequestRecord, RequestRecordProjection};
use rstest::rstest;
use std::collections::{BTreeMap, BTreeSet};

fn record(value: &str) -> RequestRecord {
    serde_json::from_str(value).unwrap()
}

fn text(value: &RequestRecord) -> String {
    serde_json::to_string(&value).unwrap()
}

#[rstest]
#[case::safe_copy_mode_and_fallback("safe_copy_mode_and_fallback")]
#[case::safe_copy_span_order("safe_copy_span_order")]
#[case::independent_snapshot("independent_snapshot")]
#[case::parameter_filter_null_and_depth("parameter_filter_null_and_depth")]
#[case::internal_parameter_names("internal_parameter_names")]
#[case::gateway_injection_bucket("gateway_injection_bucket")]
#[case::gateway_injection_attempt_scope("gateway_injection_attempt_scope")]
fn request_records_cases(#[case] name: &str) {
    match name {
        "safe_copy_mode_and_fallback" => {
            let value = record(r#"{"value":{"nested":true}}"#);
            assert_eq!(
                RequestRecordProjection::safe_copy(value.clone(), true),
                value
            );
            assert_eq!(
                text(&RequestRecordProjection::safe_copy(value, false)),
                r#"{"value":{"nested":true}}"#
            );
        }
        "safe_copy_span_order" => {
            let input = record(
                r#"{"metadata":{"litellm_parent_otel_span":"first"},"litellm_metadata":{"litellm_parent_otel_span":"last"}}"#,
            );
            assert!(
                text(&RequestRecordProjection::safe_copy(input, false))
                    .contains(r#""litellm_parent_otel_span":"placeholder""#)
            );
            let mut fields = BTreeMap::from([(
                String::from("metadata"),
                record(r#"{"litellm_parent_otel_span":"placeholder"}"#),
            )]);
            RequestRecordProjection::restore(
                &mut fields,
                "metadata",
                &RequestRecord::String(String::from("restored")),
            );
            assert_eq!(
                text(&fields["metadata"]),
                r#"{"litellm_parent_otel_span":"restored"}"#
            );
        }
        "independent_snapshot" => {
            let input =
                record(r#"{"metadata":{"litellm_parent_otel_span":"span"},"value":{"x":1}}"#);
            assert_eq!(RequestRecordProjection::independent_snapshot(&input), input);
        }
        "parameter_filter_null_and_depth" => {
            assert_eq!(
                text(&RequestRecordProjection::filter_parameters(
                    record(r#"{"keep":1,"drop":null}"#),
                    20_i32
                )),
                r#"{"keep":1}"#
            );
            assert_eq!(
                text(&RequestRecordProjection::filter_parameters(
                    record("[1,null,2]"),
                    20_i32,
                )),
                "[1,2]"
            );
            let value = record(r#"{"drop":null}"#);
            assert_eq!(
                RequestRecordProjection::filter_parameters(value.clone(), 0_i32),
                value
            );
        }
        "internal_parameter_names" => {
            let additional = BTreeSet::from([String::from("extra")]);
            let output = RequestRecordProjection::filter_internal(
                record(r#"{"keep":null,"skip_mcp_handler":true,"extra":1,"_other":2}"#),
                &additional,
            );
            assert_eq!(text(&output), r#"{"_other":2,"keep":null}"#);
        }
        "gateway_injection_bucket" => {
            let mut input = record(r#"{"metadata":{},"model_info":{"id":"deployment-a"}}"#);
            GatewayCacheAttribution::record(&mut input, 1_i32, false);
            assert!(text(&input).contains(r#""litellm_gateway_injected_cache":"deployment-a""#));
        }
        "gateway_injection_attempt_scope" => {
            let mut input = record(r#"{"metadata":{},"model_info":{"id":"deployment-a"}}"#);
            GatewayCacheAttribution::record(&mut input, 1_i32, true);
            GatewayCacheAttribution::record(&mut input, 1_i32, false);
            assert!(text(&input).contains(r#""litellm_gateway_injected_cache":"""#));
        }
        _ => {}
    }
}
