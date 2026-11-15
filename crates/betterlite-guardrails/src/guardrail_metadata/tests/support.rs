use std::collections::BTreeMap;
use std::fmt::Debug;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::content_filter::risk_scoring::GuardrailStatus;
use crate::guardrail_metadata::{RequestData, ScanEntry, logging_caching_headers, record_scan_id};
use crate::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailStatus as RecordStatus, GuardrailTracingDetail,
};
use crate::openai_moderation::logging::{GuardrailLogRecord, GuardrailResponse};
use crate::openai_moderation::wire::ModerationResponse;
use crate::panw_prisma_airs::audit::{GuardrailAudit, ScanIdEntry, Stage};

pub(super) fn data(source: &str) -> RequestData {
    serde_json::from_str(source).unwrap()
}

pub(super) fn value(source: &str) -> GuardrailValue {
    serde_json::from_str(source).unwrap()
}

pub(super) fn assert_json<T>(actual: &T, expected_json: &str)
where
    T: DeserializeOwned + Serialize + PartialEq + Debug,
{
    let reparsed: T = serde_json::from_str(&serde_json::to_string(actual).unwrap()).unwrap();
    let expected: T = serde_json::from_str(expected_json).unwrap();
    assert_eq!(reparsed, expected);
}

pub(super) fn headers(request_data: &RequestData) -> Vec<(String, String)> {
    logging_caching_headers(request_data).unwrap()
}

pub(super) fn header<'list>(headers: &'list [(String, String)], name: &str) -> Option<&'list str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, found)| found.as_str())
}

fn scan_entry(scan_id: &str) -> ScanEntry {
    ScanEntry {
        guardrail: Some("pii".to_owned()),
        stage: GuardrailEventHooks::PreCall,
        provider: "panw_prisma_airs".to_owned(),
        scan_id: scan_id.to_owned(),
    }
}

pub(super) fn scan_entry_value(scan_id: &str) -> GuardrailValue {
    serde_json::from_value(serde_json::to_value(scan_entry(scan_id)).unwrap()).unwrap()
}

pub(super) fn scan_entry_text(scan_id: &str) -> String {
    serde_json::to_string(&scan_entry(scan_id)).unwrap()
}

pub(super) fn record() -> GuardrailInvocationRecord {
    GuardrailInvocationRecord {
        guardrail_name: Some("model-armor".to_owned()),
        guardrail_provider: Some("model_armor".to_owned()),
        guardrail_mode: None,
        guardrail_response: value(r#"{"verdict": "allow"}"#),
        guardrail_status: RecordStatus::Success,
        start_time: Some(1.5),
        end_time: None,
        duration: None,
        masked_entity_count: None,
        tracing_detail: GuardrailTracingDetail::default(),
    }
}

pub(super) fn moderation_response(id: &str) -> ModerationResponse {
    ModerationResponse {
        id: id.to_owned(),
        model: "omni-moderation-latest".to_owned(),
        results: Vec::new(),
        extra: BTreeMap::new(),
    }
}

pub(super) fn moderation_record(id: &str, status: GuardrailStatus) -> GuardrailLogRecord {
    GuardrailLogRecord {
        guardrail_name: "moderation".to_owned(),
        guardrail_response: GuardrailResponse::Moderation(moderation_response(id)),
        guardrail_status: status,
        start_time: 0.0,
        end_time: 1.0,
        duration: 1.0,
        tracing_detail: None,
    }
}

pub(super) fn failure_record() -> GuardrailLogRecord {
    GuardrailLogRecord {
        guardrail_response: GuardrailResponse::Text("allow".to_owned()),
        guardrail_status: GuardrailStatus::GuardrailFailedToRespond,
        ..moderation_record("unused", GuardrailStatus::GuardrailFailedToRespond)
    }
}

pub(super) fn audit(script: &str) -> GuardrailAudit {
    let mut scan_metadata = Vec::new();
    for call in script.split(',').filter(|part| !part.is_empty()) {
        let mut parts = call.split('|');
        let stage_text = parts.next().unwrap();
        let scan_id = parts.next().unwrap();
        let guardrail = parts.next().unwrap();
        let stage = match stage_text {
            "pre" => Stage::PreCall,
            _ => Stage::PostCall,
        };
        scan_metadata.push(ScanIdEntry {
            guardrail: guardrail.to_owned(),
            stage,
            provider: "panw_prisma_airs".to_owned(),
            scan_id: scan_id.to_owned(),
        });
    }
    GuardrailAudit {
        scan_ids: Vec::new(),
        scan_metadata,
        applied_guardrails: vec!["pii".to_owned()],
        information: Vec::new(),
    }
}

pub(super) fn moderation_records(script: &str) -> Vec<GuardrailLogRecord> {
    let mut records = Vec::new();
    for call in script.split(',').filter(|part| !part.is_empty()) {
        let mut parts = call.split('|');
        let kind = parts.next().unwrap();
        let id = parts.next().unwrap();
        match kind {
            "flagged" => records.push(moderation_record(id, GuardrailStatus::GuardrailIntervened)),
            "text" => records.push(failure_record()),
            _ => records.push(moderation_record(id, GuardrailStatus::Success)),
        }
    }
    records
}

pub(super) fn apply_calls(request_data: &mut RequestData, script: &str) {
    for call in script.split(';') {
        let mut parts = call.split('|');
        let stage_text = parts.next().unwrap();
        let scan_id = parts.next().unwrap();
        let guardrail_text = parts.next().unwrap();
        let stage = match stage_text {
            "pre" => GuardrailEventHooks::PreCall,
            _ => GuardrailEventHooks::PostCall,
        };
        let guardrail = if guardrail_text == "-" {
            None
        } else {
            Some(guardrail_text)
        };
        record_scan_id(
            request_data,
            Some(scan_id),
            guardrail,
            "panw_prisma_airs",
            stage,
        );
    }
}
