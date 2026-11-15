use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::content_filter_records::BlockCodeAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::block_code_execution::registration::{
    DETECTION_METHOD, GUARDRAIL_PROVIDER,
};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordMode, GuardrailStatus,
};

use crate::block_code_execution::support::{
    apply_capturing, guardrail, inputs, params, request_data,
};

fn named_response_record() -> GuardrailInvocationRecord {
    let mut configured = params();
    configured.guardrail_name = Some("mine".to_owned());
    configured.blocked_languages = Some(vec!["python".to_owned()]);
    let built = guardrail(configured);
    let mut data = request_data();
    let (_outcome, mut records) = apply_capturing(
        &built,
        inputs(&["```rust\nfn main() {}\n```"]),
        &mut data,
        GuardrailInputType::Response,
    );
    records.remove(0)
}

#[test]
fn a_successful_scan_records_the_detection_list() {
    let record = named_response_record();
    let GuardrailValue::Array(detections) = &record.guardrail_response else {
        panic!("a scan record carries its detections");
    };
    assert_eq!(detections.len(), 1);
    let GuardrailValue::Object(detection) = &detections[0] else {
        panic!("a detection is an object");
    };
    assert_eq!(
        detection.get("type"),
        Some(&GuardrailValue::Text("code_block".to_owned()))
    );
    assert_eq!(
        detection.get("language"),
        Some(&GuardrailValue::Text("rust".to_owned()))
    );
    assert_eq!(
        detection.get("action_taken"),
        Some(&GuardrailValue::Text("allow".to_owned()))
    );
    assert_eq!(
        detection.get("confidence"),
        Some(&GuardrailValue::Decimal(0.0))
    );
}

#[test]
fn a_successful_scan_records_provider_mode_and_tracing_detail() {
    let record = named_response_record();
    assert_eq!(record.guardrail_status, GuardrailStatus::Success);
    assert_eq!(
        record.guardrail_provider.as_deref(),
        Some(GUARDRAIL_PROVIDER)
    );
    assert_eq!(
        record.guardrail_mode,
        Some(GuardrailRecordMode::Event(GuardrailEventHooks::PostCall))
    );
    assert_eq!(
        record.tracing_detail.detection_method.as_deref(),
        Some(DETECTION_METHOD)
    );
    assert_eq!(record.guardrail_name.as_deref(), Some("mine"));
    assert_eq!(record.tracing_detail.guardrail_id.as_deref(), Some("mine"));
    assert_eq!(
        record.tracing_detail.confidence_score.map(f64::to_bits),
        Some(0.0_f64.to_bits())
    );
    assert_eq!(
        record.tracing_detail.match_details,
        Some(record.guardrail_response.clone())
    );
}

#[test]
fn a_request_scan_records_the_pre_call_event() {
    let built = guardrail(params());
    let mut data = request_data();
    let (_outcome, records) = apply_capturing(
        &built,
        inputs(&["plain text"]),
        &mut data,
        GuardrailInputType::Request,
    );
    assert_eq!(
        records[0].guardrail_mode,
        Some(GuardrailRecordMode::Event(GuardrailEventHooks::PreCall))
    );
}

#[test]
fn an_empty_scan_records_an_empty_response_and_no_confidence() {
    let built = guardrail(params());
    let mut data = request_data();
    let (_outcome, records) =
        apply_capturing(&built, inputs(&[]), &mut data, GuardrailInputType::Request);
    let record = &records[0];
    assert_eq!(record.guardrail_response, GuardrailValue::Array(Vec::new()));
    assert!(record.tracing_detail.confidence_score.is_none());
    assert_eq!(
        record.tracing_detail.match_details,
        Some(GuardrailValue::Array(Vec::new()))
    );
    assert!(record.start_time.is_some());
    assert!(record.end_time.is_some());
    assert!(record.duration.is_some());
}

#[test]
fn the_confidence_score_is_the_maximum_across_detections() {
    let mut configured = params();
    configured.action = BlockCodeAction::Mask;
    let built = guardrail(configured);
    let mut data = request_data();
    let (_outcome, records) = apply_capturing(
        &built,
        inputs(&["```python\na\n```\n```text\nb\n```"]),
        &mut data,
        GuardrailInputType::Response,
    );
    let record = &records[0];
    assert_eq!(
        record.tracing_detail.confidence_score.map(f64::to_bits),
        Some(1.0_f64.to_bits())
    );
    let GuardrailValue::Array(detections) = &record.guardrail_response else {
        panic!("a scan record carries its detections");
    };
    assert_eq!(detections.len(), 2);
}

#[test]
fn the_timing_comes_from_the_injected_clock() {
    static TICKS: AtomicU64 = AtomicU64::new(0);

    fn clock() -> SystemTime {
        let seconds = TICKS.fetch_add(1, Ordering::SeqCst);
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    let mut configured = params();
    configured.clock = clock;
    let built = guardrail(configured);
    let mut data = request_data();
    let (_outcome, records) = apply_capturing(
        &built,
        inputs(&["plain text"]),
        &mut data,
        GuardrailInputType::Request,
    );
    let record = &records[0];
    assert_eq!(record.start_time.map(f64::to_bits), Some(0.0_f64.to_bits()));
    assert_eq!(record.end_time.map(f64::to_bits), Some(1.0_f64.to_bits()));
    assert_eq!(record.duration.map(f64::to_bits), Some(1.0_f64.to_bits()));
}

#[test]
fn the_record_lands_in_the_request_metadata() {
    let built = guardrail(params());
    let mut data = request_data();
    let (_outcome, _records) = apply_capturing(
        &built,
        inputs(&["plain text"]),
        &mut data,
        GuardrailInputType::Request,
    );
    let Some(GuardrailValue::Object(metadata)) = data.get("metadata") else {
        panic!("the metadata bucket holds the record");
    };
    let Some(GuardrailValue::Array(entries)) =
        metadata.get("standard_logging_guardrail_information")
    else {
        panic!("the record sits under its standard logging key");
    };
    assert_eq!(entries.len(), 1);
    let GuardrailValue::Object(entry) = &entries[0] else {
        panic!("a record entry is an object");
    };
    assert_eq!(
        entry.get("guardrail_provider"),
        Some(&GuardrailValue::Text(GUARDRAIL_PROVIDER.to_owned()))
    );
    assert_eq!(
        entry.get("guardrail_status"),
        Some(&GuardrailValue::Text("success".to_owned()))
    );
    assert_eq!(
        entry.get("detection_method"),
        Some(&GuardrailValue::Text(DETECTION_METHOD.to_owned()))
    );
    assert_eq!(
        entry.get("guardrail_mode"),
        Some(&GuardrailValue::Text("pre_call".to_owned()))
    );
}
