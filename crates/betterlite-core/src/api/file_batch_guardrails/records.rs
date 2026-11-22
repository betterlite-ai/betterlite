use serde::Serialize;
use thiserror::Error;

use super::SUMMARY_LIMIT;
use super::custom_id::describe;
use super::record_json::RecordObject;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnparseableRecord {
    pub line_number: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnscannableRecord {
    pub line_number: usize,
    pub custom_id: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnroutableRecord {
    pub line_number: usize,
    pub custom_id: Option<String>,
    pub guardrail: Option<String>,
}

// Each message is the detail.error of the HTTPException(status_code=400) raise_public raises
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BatchScanFailure {
    #[error(
        "The 'body' of batch input line {} is not an object, so guardrails cannot be applied to it",
        .0.line_number
    )]
    Unparseable(UnparseableRecord),
    #[error(
        "Batch input line {}{} targets {} and its body has no messages, prompt or input, so guardrails cannot read it. Give the record a chat, completion, embedding, responses or messages body",
        .0.line_number,
        describe(.0.custom_id.as_deref()),
        .0.url.as_deref().filter(|url| !url.is_empty()).unwrap_or("no url")
    )]
    Unscannable(UnscannableRecord),
    #[error(
        "Batch input line {}{} was routed to a different model by {}, and every record of a batch file goes to one provider, so the file cannot be submitted. Send that record outside the batch",
        .0.line_number,
        describe(.0.custom_id.as_deref()),
        .0.guardrail.as_deref().filter(|name| !name.is_empty()).unwrap_or("a guardrail")
    )]
    Unroutable(UnroutableRecord),
}

// The 400 the files endpoint answers with, as {"error": <message>} in its detail
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BatchRejection {
    #[error(transparent)]
    Failure(BatchScanFailure),
    #[error(
        "Every record in the batch input file was blocked by a guardrail, so there is nothing left to submit"
    )]
    NothingToSubmit,
}

impl BatchRejection {
    pub const STATUS: u16 = 400;
}

// A rewritten record on its way to the scan spool, held only for the window it was scanned in
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redaction {
    pub line_number: usize,
    pub custom_id: Option<String>,
    pub text: String,
}

// Where the re-serialized record sits in the scan spool, so a large file's rewrites stay off the heap
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRedacted {
    pub line_number: usize,
    pub custom_id: Option<String>,
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordDropped {
    pub line_number: usize,
    pub custom_id: Option<String>,
    pub guardrail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordChange {
    Redacted(RecordRedacted),
    Dropped(RecordDropped),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanOutcome {
    Failure(BatchScanFailure),
    Redaction(Redaction),
    Dropped(RecordDropped),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardrailAction {
    Redacted,
    Dropped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BatchGuardrailRecord {
    pub line: usize,
    pub custom_id: Option<String>,
    pub action: GuardrailAction,
    pub guardrail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BatchGuardrailReport {
    pub submitted_records: usize,
    pub modified_records: Vec<BatchGuardrailRecord>,
}

// What the scan decided per record; no changes means the upload proceeds untouched
#[derive(Debug)]
pub struct BatchScanResult<Spool> {
    pub changes: Vec<RecordChange>,
    pub scanned_records: usize,
    pub redactions: Spool,
}

impl<Spool> BatchScanResult<Spool> {
    #[must_use]
    pub fn submitted_records(&self) -> usize {
        let dropped = self
            .changes
            .iter()
            .filter(|change| matches!(change, RecordChange::Dropped(_)))
            .count();
        self.scanned_records.saturating_sub(dropped)
    }

    // The server-side log line, capped so one upload cannot flood it
    #[must_use]
    pub fn summary(&self) -> String {
        let shown: Vec<String> = self
            .changes
            .iter()
            .take(SUMMARY_LIMIT)
            .map(|change| {
                let record = change.guardrail_record();
                let action = match record.action {
                    GuardrailAction::Redacted => "redacted",
                    GuardrailAction::Dropped => "dropped",
                };
                format!(
                    "line {}{} {action}",
                    record.line,
                    describe(record.custom_id.as_deref())
                )
            })
            .collect();
        let joined = shown.join(", ");
        match self.changes.len().saturating_sub(SUMMARY_LIMIT) {
            0 => joined,
            remaining => format!("{joined}, and {remaining} more"),
        }
    }

    #[must_use]
    pub fn report(&self) -> BatchGuardrailReport {
        BatchGuardrailReport {
            submitted_records: self.submitted_records(),
            modified_records: self
                .changes
                .iter()
                .map(RecordChange::guardrail_record)
                .collect(),
        }
    }
}

impl RecordChange {
    // The public projection: a redaction's spool offsets stay internal
    #[must_use]
    pub fn guardrail_record(&self) -> BatchGuardrailRecord {
        match self {
            Self::Redacted(redacted) => BatchGuardrailRecord {
                line: redacted.line_number,
                custom_id: redacted.custom_id.clone(),
                action: GuardrailAction::Redacted,
                guardrail: None,
            },
            Self::Dropped(dropped) => BatchGuardrailRecord {
                line: dropped.line_number,
                custom_id: dropped.custom_id.clone(),
                action: GuardrailAction::Dropped,
                guardrail: dropped.guardrail.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRecord {
    pub line_number: usize,
    pub payload: RecordObject,
}
