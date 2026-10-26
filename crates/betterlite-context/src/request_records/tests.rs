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

fn handle(kind: super::HandleKind, identity: u64) -> RequestRecord {
    RequestRecord::Handle(super::record::InternalHandle { kind, identity })
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
            let mut value = record(r#"{"value":{"nested":true}}"#);
            let original: *const RequestRecord = &raw const value;
            let borrowed = RequestRecordProjection::safe_copy(&mut value, true).unwrap();
            let borrowed_pointer: *const RequestRecord = borrowed.as_ref();
            assert_eq!(borrowed_pointer, original);
            assert_eq!(
                text(
                    RequestRecordProjection::safe_copy(&mut value, false)
                        .unwrap()
                        .as_ref()
                ),
                r#"{"value":{"nested":true}}"#
            );
        }
        "safe_copy_span_order" => {
            let mut span_input = record(
                r#"{"metadata":{"litellm_parent_otel_span":"first"},"litellm_metadata":{"litellm_parent_otel_span":"last"}}"#,
            );
            let copied = RequestRecordProjection::safe_copy(&mut span_input, false).unwrap();
            assert_eq!(
                text(&copied),
                r#"{"litellm_metadata":{"litellm_parent_otel_span":"placeholder"},"metadata":{"litellm_parent_otel_span":"placeholder"}}"#
            );
            assert_eq!(
                text(&span_input),
                r#"{"litellm_metadata":{"litellm_parent_otel_span":"last"},"metadata":{"litellm_parent_otel_span":"last"}}"#
            );
            let mut null_span = record(r#"{"metadata":{"litellm_parent_otel_span":null}}"#);
            RequestRecordProjection::safe_copy(&mut null_span, false).unwrap();
            assert_eq!(
                text(&null_span),
                r#"{"metadata":{"litellm_parent_otel_span":"placeholder"}}"#
            );
            let mut invalid = record(r#"{"metadata":null}"#);
            let error = RequestRecordProjection::safe_copy(&mut invalid, false);
            assert!(matches!(
                error,
                Err(super::RequestRecordError::MetadataMembership { .. })
            ));
            let mut string_without_span = record(r#"{"metadata":"ordinary"}"#);
            let string_copy = RequestRecordProjection::safe_copy(&mut string_without_span, false);
            if let Err(copy_error) = string_copy {
                panic!("ordinary string metadata should copy: {copy_error}");
            }
            let mut string_with_span = record(r#"{"metadata":"litellm_parent_otel_span"}"#);
            assert!(matches!(
                RequestRecordProjection::safe_copy(&mut string_with_span, false),
                Err(super::RequestRecordError::MetadataMutation { .. })
            ));
            let mut array_with_span = record(r#"{"metadata":["litellm_parent_otel_span"]}"#);
            assert!(matches!(
                RequestRecordProjection::safe_copy(&mut array_with_span, false),
                Err(super::RequestRecordError::MetadataMutation { .. })
            ));
            let mut array_without_span = record(r#"{"metadata":["ordinary"]}"#);
            assert!(RequestRecordProjection::safe_copy(&mut array_without_span, false).is_ok());
        }
        "independent_snapshot" => {
            let input =
                record(r#"{"metadata":{"litellm_parent_otel_span":"span"},"value":{"x":1}}"#);
            assert_eq!(RequestRecordProjection::independent_snapshot(&input), input);
            assert_eq!(
                RequestRecordProjection::independent_snapshot(&record(r#"{"metadata":{}}"#)),
                record(r#"{"metadata":{}}"#)
            );
            assert_eq!(
                RequestRecordProjection::independent_snapshot(&record(
                    r#"{"metadata":{"other":true}}"#
                )),
                record(r#"{"metadata":{"other":true}}"#)
            );
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
            let handles = RequestRecord::Object(BTreeMap::from([
                (
                    "exception".to_owned(),
                    handle(super::HandleKind::Exception, 1),
                ),
                (
                    "callable".to_owned(),
                    handle(super::HandleKind::Callable, 2),
                ),
                ("logging".to_owned(), handle(super::HandleKind::Logging, 3)),
                ("router".to_owned(), handle(super::HandleKind::Router, 4)),
                ("opaque".to_owned(), handle(super::HandleKind::Opaque, 5)),
            ]));
            assert_eq!(
                RequestRecordProjection::filter_parameters(handles, 20_i32),
                RequestRecord::Object(BTreeMap::from([(
                    "opaque".to_owned(),
                    handle(super::HandleKind::Opaque, 5)
                )]))
            );
        }
        "gateway_injection_bucket" => {
            let mut input = record(
                r#"{"litellm_metadata":{},"metadata":{"old":true},"model_info":{"id":"deployment-a"}}"#,
            );
            GatewayCacheAttribution::record(&mut input, 1_i32, false);
            assert_eq!(
                text(&input),
                r#"{"litellm_metadata":{"litellm_gateway_injected_cache":"deployment-a"},"metadata":{"old":true},"model_info":{"id":"deployment-a"}}"#
            );
            let mut missing = record(r#"{"model_info":{"id":"deployment-a"}}"#);
            GatewayCacheAttribution::record(&mut missing, 1_i32, false);
            assert_eq!(missing, record(r#"{"model_info":{"id":"deployment-a"}}"#));
            let mut explicit_null = record(r#"{"metadata":{},"model_info":{"id":null}}"#);
            GatewayCacheAttribution::record(&mut explicit_null, 1_i32, false);
            assert_eq!(
                text(&explicit_null),
                r#"{"metadata":{"litellm_gateway_injected_cache":null},"model_info":{"id":null}}"#
            );
            let mut nondict = record(r#"{"metadata":{},"model_info":"wrong"}"#);
            GatewayCacheAttribution::record(&mut nondict, 1_i32, false);
            assert_eq!(
                text(&nondict),
                r#"{"metadata":{"litellm_gateway_injected_cache":""},"model_info":"wrong"}"#
            );
            let mut sentinel = record(
                r#"{"metadata":{"litellm_gateway_injected_cache":""},"model_info":{"id":"deployment-a"}}"#,
            );
            GatewayCacheAttribution::record(&mut sentinel, 1_i32, false);
            assert_eq!(
                text(&sentinel),
                r#"{"metadata":{"litellm_gateway_injected_cache":""},"model_info":{"id":"deployment-a"}}"#
            );
        }
        "gateway_injection_attempt_scope" => {
            let mut input = record(r#"{"metadata":{},"model_info":{"id":"deployment-a"}}"#);
            GatewayCacheAttribution::record(&mut input, 1_i32, false);
            input
                .object_mut()
                .unwrap()
                .insert("model_info".to_owned(), record(r#"{"id":"deployment-b"}"#));
            GatewayCacheAttribution::record(&mut input, 1_i32, false);
            assert_eq!(
                text(&input),
                r#"{"metadata":{"litellm_gateway_injected_cache":"deployment-b"},"model_info":{"id":"deployment-b"}}"#
            );
            GatewayCacheAttribution::record(&mut input, 0_i32, false);
            GatewayCacheAttribution::record(&mut input, -1_i32, false);
            assert_eq!(
                text(&input),
                r#"{"metadata":{"litellm_gateway_injected_cache":"deployment-b"},"model_info":{"id":"deployment-b"}}"#
            );
            GatewayCacheAttribution::record(&mut input, 1_i32, true);
            GatewayCacheAttribution::record(&mut input, 1_i32, false);
            assert_eq!(
                text(&input),
                r#"{"metadata":{"litellm_gateway_injected_cache":""},"model_info":{"id":"deployment-b"}}"#
            );
        }
        other => panic!("unexpected request-record case: {other}"),
    }
}
