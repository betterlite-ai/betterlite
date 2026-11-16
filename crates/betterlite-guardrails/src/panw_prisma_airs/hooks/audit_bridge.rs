use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Serialize;

use super::data::{Data, LITELLM_METADATA, METADATA};
use crate::guardrail_metadata::merge_prisma_audit;
use crate::panw_prisma_airs::audit::{
    Direction, GuardrailAudit, GuardrailInformation, GuardrailStatus, Stage,
};
use crate::panw_prisma_airs::construct::PanwPrismaAirs;
use crate::panw_prisma_airs::error_handling::record_scan_id;
use crate::panw_prisma_airs::verdict::{Action, Failure, ScanVerdict, VerdictSummary};
use crate::tool_permission_requests::header_mutation::record_applied_guardrail;

const RECORDS_KEY: &str = "standard_logging_guardrail_information";
// The name B3786's error handling records; its constant is private to that module.
pub const PROVIDER_NAME: &str = "panw_prisma_airs";

#[derive(Debug, Clone, Copy)]
pub struct ScanRecord<'scan> {
    pub verdict: &'scan ScanVerdict,
    pub start_time: SystemTime,
    pub direction: Direction,
}

// Mirrors LiteLLM's add_standard_logging_guardrail_information_to_request_data for a successful scan.
pub fn record_information(
    guardrail: &PanwPrismaAirs,
    audit: &mut GuardrailAudit,
    record: ScanRecord<'_>,
) {
    let end_time = (guardrail.now)();
    let status = match record.verdict.action {
        Action::Allow => GuardrailStatus::Success,
        Action::Block => GuardrailStatus::GuardrailIntervened,
    };
    audit.information.push(GuardrailInformation {
        provider: PROVIDER_NAME.to_owned(),
        status,
        response: record.verdict.summary(),
        start_time: record.start_time,
        end_time,
        duration: end_time
            .duration_since(record.start_time)
            .unwrap_or_default(),
        event_type: record.direction.stage(),
    });
}

// The three writes LiteLLM repeats after a successful response scan: record, scan id, applied header.
pub fn record_response_success(
    guardrail: &PanwPrismaAirs,
    audit: &mut GuardrailAudit,
    verdict: &ScanVerdict,
    start_time: SystemTime,
) {
    record_information(
        guardrail,
        audit,
        ScanRecord {
            verdict,
            start_time,
            direction: Direction::Response,
        },
    );
    record_scan_id(guardrail, audit, verdict, Direction::Response);
    audit.add_applied_guardrail(&guardrail.settings.guardrail_name);
}

// The hooks drain the audit into the request the way LiteLLM's helpers write it as they run.
pub fn flush(data: &mut Data, audit: &GuardrailAudit, guardrail_name: &str) {
    merge_prisma_audit(data, audit);
    for name in &audit.applied_guardrails {
        record_applied_guardrail(data, name);
    }
    for information in &audit.information {
        append_information(data, guardrail_name, information);
    }
}

#[derive(Serialize)]
struct StandardRecord<'record> {
    guardrail_name: &'record str,
    guardrail_provider: &'record str,
    guardrail_mode: Stage,
    guardrail_response: GuardrailValue,
    guardrail_status: GuardrailStatus,
    start_time: f64,
    end_time: f64,
    duration: f64,
    masked_entity_count: Option<BTreeMap<String, i64>>,
}

fn append_information(data: &mut Data, guardrail_name: &str, information: &GuardrailInformation) {
    let record = StandardRecord {
        guardrail_name,
        guardrail_provider: &information.provider,
        guardrail_mode: information.event_type,
        guardrail_response: verdict_value(&information.response),
        guardrail_status: information.status,
        start_time: seconds(information.start_time),
        end_time: seconds(information.end_time),
        duration: information.duration.as_secs_f64(),
        masked_entity_count: None,
    };
    let encoded = serde_json::to_value(&record)
        .and_then(|value| serde_json::from_value::<GuardrailValue>(value));
    match encoded {
        Ok(value) => append_unique(data, value),
        Err(source) => {
            tracing::error!("PANW Prisma AIRS: guardrail information is not JSON: {source}");
        }
    }
}

fn append_unique(data: &mut Data, value: GuardrailValue) {
    let key = if data.contains_key(LITELLM_METADATA) {
        LITELLM_METADATA
    } else {
        METADATA
    };
    let mut bucket = match data.remove(key) {
        Some(GuardrailValue::Object(fields)) => fields,
        None | Some(_) => BTreeMap::new(),
    };
    let mut entries = match bucket.remove(RECORDS_KEY) {
        None | Some(GuardrailValue::Null) => Vec::new(),
        Some(GuardrailValue::Array(entries)) => entries,
        Some(other) => vec![other],
    };
    if !entries.contains(&value) {
        entries.push(value);
    }
    bucket.insert(RECORDS_KEY.to_owned(), GuardrailValue::Array(entries));
    data.insert(key.to_owned(), GuardrailValue::Object(bucket));
}

// The scan result dict LiteLLM logs, including the failure marker its synthetic results carry.
fn verdict_value(summary: &VerdictSummary) -> GuardrailValue {
    let mut fields = BTreeMap::new();
    let action = match summary.action {
        Action::Allow => "allow",
        Action::Block => "block",
    };
    fields.insert("action".to_owned(), GuardrailValue::Text(action.to_owned()));
    if let Some(category) = &summary.category {
        fields.insert(
            "category".to_owned(),
            GuardrailValue::Text(category.clone()),
        );
    }
    match summary.failure {
        Some(Failure::Permanent) => {
            fields.insert("_always_block".to_owned(), GuardrailValue::Boolean(true));
        }
        Some(Failure::Transient) => {
            fields.insert("_is_transient".to_owned(), GuardrailValue::Boolean(true));
        }
        None => {}
    }
    for (key, raw) in &summary.extra {
        match serde_json::from_str::<GuardrailValue>(raw.get()) {
            Ok(value) => {
                fields.insert(key.clone(), value);
            }
            Err(source) => {
                tracing::warn!("PANW Prisma AIRS: scan field {key} cannot be logged: {source}");
            }
        }
    }
    GuardrailValue::Object(fields)
}

fn seconds(time: SystemTime) -> f64 {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs_f64(),
        Err(_) => 0.0,
    }
}
