use betterlite_core::api::file_batch_guardrails::records::{
    BatchRejection, BatchScanFailure, BatchScanResult, RecordChange, RecordDropped, RecordRedacted,
    UnparseableRecord, UnroutableRecord, UnscannableRecord,
};
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::{Action, Item};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Failure {
    Unparseable,
    Unscannable,
    Unroutable,
    NothingToSubmit,
}

pub(super) fn report(scanned_records: usize, changes: &[Item<Action>]) -> HelperResult {
    let result = BatchScanResult {
        changes: changes.iter().map(change).collect(),
        scanned_records,
        redactions: (),
    };
    let report = serde_json::to_value(result.report()).or_value_error()?;
    let submitted = result.submitted_records();
    Ok(json!({"submitted_records": submitted, "report": report, "summary": result.summary()}))
}

fn change(item: &Item<Action>) -> RecordChange {
    let line_number = item.line_number;
    let custom_id = item.custom_id.clone();
    match item.kind {
        Action::Redacted => RecordChange::Redacted(RecordRedacted {
            line_number,
            custom_id,
            offset: item.offset,
            length: item.length,
        }),
        Action::Dropped => RecordChange::Dropped(RecordDropped {
            line_number,
            custom_id,
            guardrail: item.guardrail_name.clone(),
        }),
    }
}

// The failure raise_public and raise_nothing_to_submit answer for
pub(super) fn rejection(item: &Item<Failure>) -> BatchRejection {
    let line_number = item.line_number;
    let custom_id = item.custom_id.clone();
    let failure = match item.kind {
        Failure::Unparseable => BatchScanFailure::Unparseable(UnparseableRecord { line_number }),
        Failure::Unscannable => BatchScanFailure::Unscannable(UnscannableRecord {
            line_number,
            custom_id,
            url: item.url.clone(),
        }),
        Failure::Unroutable => BatchScanFailure::Unroutable(UnroutableRecord {
            line_number,
            custom_id,
            guardrail: item.guardrail_name.clone(),
        }),
        Failure::NothingToSubmit => return BatchRejection::NothingToSubmit,
    };
    BatchRejection::Failure(failure)
}
