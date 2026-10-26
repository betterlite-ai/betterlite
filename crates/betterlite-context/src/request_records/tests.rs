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
            let mut value = record(r#"{"value":{"nested":true}}"#);
            assert_eq!(
                RequestRecordProjection::safe_copy(&mut value, true).unwrap(),
                value
            );
            assert_eq!(
                text(&RequestRecordProjection::safe_copy(&mut value, false).unwrap()),
                r#"{"value":{"nested":true}}"#
            );
            assert_eq!(value, record(r#"{"value":{"nested":true}}"#));
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
            assert_eq!(
                RequestRecordProjection::safe_copy(&mut invalid, false),
                Err(super::RequestRecordError::NonObjectMetadata {
                    bucket: "metadata".to_owned()
                })
            );
        }
        "independent_snapshot" => {
            let input =
                record(r#"{"metadata":{"litellm_parent_otel_span":"span"},"value":{"x":1}}"#);
            assert_eq!(RequestRecordProjection::independent_snapshot(&input), input);
            let handle_input = RequestRecord::Object(BTreeMap::from([
                (
                    "metadata".to_owned(),
                    RequestRecord::Object(BTreeMap::from([(
                        String::from("litellm_parent_otel_span"),
                        RequestRecord::Handle(super::InternalHandle::new(
                            super::HandleKind::Opaque,
                            7,
                        )),
                    )])),
                ),
                (
                    "litellm_metadata".to_owned(),
                    RequestRecord::Object(BTreeMap::from([(
                        String::from("litellm_parent_otel_span"),
                        RequestRecord::Handle(super::InternalHandle::new(
                            super::HandleKind::Opaque,
                            8,
                        )),
                    )])),
                ),
            ]));
            let snapshot = RequestRecordProjection::independent_snapshot(&handle_input);
            assert_eq!(
                handle_input,
                RequestRecordProjection::independent_snapshot(&handle_input)
            );
            let RequestRecord::Object(snapshot_fields) = snapshot else {
                panic!("snapshot must remain an object")
            };
            let RequestRecord::Object(metadata) = &snapshot_fields["metadata"] else {
                panic!("metadata must remain an object")
            };
            let RequestRecord::Handle(metadata_span) = &metadata["litellm_parent_otel_span"] else {
                panic!("metadata span must remain a handle")
            };
            assert_eq!(metadata_span.identity(), 7);
            let RequestRecord::Object(internal) = &snapshot_fields["litellm_metadata"] else {
                panic!("litellm metadata must remain an object")
            };
            let RequestRecord::Handle(internal_span) = &internal["litellm_parent_otel_span"] else {
                panic!("internal span must remain a handle")
            };
            assert_eq!(internal_span.identity(), 8);
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
                    RequestRecord::Handle(super::InternalHandle::new(
                        super::HandleKind::Exception,
                        1,
                    )),
                ),
                (
                    "callable".to_owned(),
                    RequestRecord::Handle(super::InternalHandle::new(
                        super::HandleKind::Callable,
                        2,
                    )),
                ),
                (
                    "logging".to_owned(),
                    RequestRecord::Handle(super::InternalHandle::new(
                        super::HandleKind::Logging,
                        3,
                    )),
                ),
                (
                    "router".to_owned(),
                    RequestRecord::Handle(super::InternalHandle::new(super::HandleKind::Router, 4)),
                ),
                (
                    "opaque".to_owned(),
                    RequestRecord::Handle(super::InternalHandle::new(super::HandleKind::Opaque, 5)),
                ),
            ]));
            let filtered = RequestRecordProjection::filter_parameters(handles, 20_i32);
            let RequestRecord::Object(filtered_fields) = filtered else {
                panic!("filtered handles must remain an object")
            };
            assert!(!filtered_fields.contains_key("exception"));
            assert!(!filtered_fields.contains_key("callable"));
            assert!(!filtered_fields.contains_key("logging"));
            assert!(!filtered_fields.contains_key("router"));
            assert_eq!(
                filtered_fields.get("opaque"),
                Some(&RequestRecord::Handle(super::InternalHandle::new(
                    super::HandleKind::Opaque,
                    5
                )))
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
        other => assert!(matches!(
            other,
            "safe_copy_mode_and_fallback"
                | "safe_copy_span_order"
                | "independent_snapshot"
                | "parameter_filter_null_and_depth"
                | "internal_parameter_names"
                | "gateway_injection_bucket"
                | "gateway_injection_attempt_scope"
        )),
    }
    assert!(matches!(
        name,
        "safe_copy_mode_and_fallback"
            | "safe_copy_span_order"
            | "independent_snapshot"
            | "parameter_filter_null_and_depth"
            | "internal_parameter_names"
            | "gateway_injection_bucket"
            | "gateway_injection_attempt_scope"
    ));
}
