use std::io::{BufRead, Seek};

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_models::decisions::Json;
use futures_util::future::join_all;
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;

use super::error::ScanFileError;
use super::rewind::{RewindOnDrop, settle_spool};
use super::spool::{OwnedSpool, Spool};
use super::worst::{ScanProblem, worst_problem};
use crate::api::file_batch_guardrails::SCAN_WINDOW;
use crate::api::file_batch_guardrails::fingerprint::build_scan_metadata;
use crate::api::file_batch_guardrails::hook::GuardrailsOnlyPreCall;
use crate::api::file_batch_guardrails::lines::iter_records;
use crate::api::file_batch_guardrails::records::{
    BatchScanFailure, BatchScanResult, ParsedRecord, RecordChange, RecordRedacted, ScanOutcome,
};
use crate::api::file_batch_guardrails::scan_record::{ScanContext, scan_record};
use crate::routes::catalog::RouteCatalog;

// What one scan needs from its caller: the seekable upload body, the request's context, the port for disk work, and the spool it fills
pub struct BatchScanCall<'call, Source, Hook> {
    pub file_source: &'call mut Source,
    pub request_metadata: &'call IndexMap<String, Json>,
    pub user_api_key_dict: &'call UserAPIKeyAuth,
    pub routes: &'call RouteCatalog,
    pub proxy_logging_obj: &'call Hook,
    pub tasks: &'call TaskTracker,
    pub redactions: Spool,
}

// The scan's decision: the changes to report, the file's one public refusal, or the exception the chain raised unchanged
#[derive(Debug)]
pub enum ScanFile<Spool, Raised> {
    Scanned(BatchScanResult<Spool>),
    Rejected(BatchScanFailure),
    Raised(Raised),
}

pub async fn scan_batch_input_file<Source, Hook>(
    call: BatchScanCall<'_, Source, Hook>,
) -> Result<ScanFile<Spool, Hook::Failure>, ScanFileError>
where
    Source: BufRead + Seek + Send,
    Hook: GuardrailsOnlyPreCall,
{
    let redactions = call.redactions;
    let scan_metadata = build_scan_metadata(call.request_metadata);
    let context = ScanContext {
        scan_metadata: &scan_metadata,
        user_api_key_dict: call.user_api_key_dict,
        proxy_logging_obj: call.proxy_logging_obj,
        routes: call.routes,
    };
    let mut guarded = RewindOnDrop {
        source: call.file_source,
        attempted: false,
    };
    let mut close_on_drop = OwnedSpool(Some(redactions.clone()));
    let scanned = scan_windows(&mut guarded.source, &context, call.tasks, &redactions).await;
    let progress = settle_spool(&mut guarded, &mut close_on_drop, scanned)?;
    Ok(finish_scan(progress, redactions))
}

// The file's one worst problem wins; otherwise the changes stand and the spool stays open only when there are changes
fn finish_scan<Raised>(
    mut progress: Progress<Raised>,
    redactions: Spool,
) -> ScanFile<Spool, Raised> {
    if let Some(index) = worst_problem(&progress.problems) {
        let (_, problem) = progress.problems.swap_remove(index);
        redactions.close();
        return match problem {
            ScanProblem::Raised(failure) => ScanFile::Raised(failure),
            ScanProblem::Failure(failure) => ScanFile::Rejected(failure),
        };
    }
    if progress.changes.is_empty() {
        redactions.close();
    }
    ScanFile::Scanned(BatchScanResult {
        changes: progress.changes,
        scanned_records: progress.scanned_records,
        redactions,
    })
}

// What one window's records left behind, before the file's one worst problem is picked
struct Progress<Raised> {
    changes: Vec<RecordChange>,
    scanned_records: usize,
    problems: Vec<(usize, ScanProblem<Raised>)>,
}

// A record's answer once it is paired back with its physical line number
enum WindowOutcome<Raised> {
    Outcome(ScanOutcome),
    Raised(Raised),
}

// Reads at most one window ahead, waits for every started record, and stops reading once a window failed
async fn scan_windows<Source, Hook>(
    source: &mut Source,
    context: &ScanContext<'_, Hook>,
    tasks: &TaskTracker,
    spool: &Spool,
) -> Result<Progress<Hook::Failure>, ScanFileError>
where
    Source: BufRead,
    Hook: GuardrailsOnlyPreCall,
{
    let mut progress = Progress {
        changes: Vec::new(),
        scanned_records: 0,
        problems: Vec::new(),
    };
    let mut window: Vec<ParsedRecord> = Vec::with_capacity(SCAN_WINDOW);
    for record in iter_records(&mut *source) {
        window.push(record?);
        if window.len() < SCAN_WINDOW {
            continue;
        }
        drain_window(&mut progress, &mut window, context, tasks, spool).await?;
        if !progress.problems.is_empty() {
            break;
        }
    }
    if progress.problems.is_empty() {
        drain_window(&mut progress, &mut window, context, tasks, spool).await?;
    }
    Ok(progress)
}

async fn drain_window<Hook>(
    progress: &mut Progress<Hook::Failure>,
    window: &mut Vec<ParsedRecord>,
    context: &ScanContext<'_, Hook>,
    tasks: &TaskTracker,
    spool: &Spool,
) -> Result<(), ScanFileError>
where
    Hook: GuardrailsOnlyPreCall,
{
    progress.scanned_records = progress.scanned_records.saturating_add(window.len());
    let outcomes = scan_window(window, context).await;
    for (line_number, outcome) in outcomes {
        note_outcome(progress, outcome, line_number, tasks, spool).await?;
    }
    window.clear();
    Ok(())
}

// A rewrite is spooled and becomes a change, a drop is a change, and a refusal or an exception is a problem
async fn note_outcome<Raised>(
    progress: &mut Progress<Raised>,
    outcome: WindowOutcome<Raised>,
    line_number: usize,
    tasks: &TaskTracker,
    spool: &Spool,
) -> Result<(), ScanFileError> {
    match outcome {
        WindowOutcome::Outcome(ScanOutcome::Redaction(redaction)) => {
            let length = u64::try_from(redaction.text.len())
                .map_err(|source| ScanFileError::Length { source })?;
            let offset = spool.append(tasks, redaction.text.into_bytes()).await?;
            let change = RecordRedacted {
                line_number,
                custom_id: redaction.custom_id,
                offset,
                length,
            };
            progress.changes.push(RecordChange::Redacted(change));
        }
        WindowOutcome::Outcome(ScanOutcome::Dropped(dropped)) => {
            progress.changes.push(RecordChange::Dropped(dropped));
        }
        WindowOutcome::Outcome(ScanOutcome::Failure(failure)) => {
            progress
                .problems
                .push((line_number, ScanProblem::Failure(failure)));
        }
        WindowOutcome::Raised(failure) => {
            progress
                .problems
                .push((line_number, ScanProblem::Raised(failure)));
        }
    }
    Ok(())
}

// Every record of the window is polled before any of them is answered, and the answers keep the file's row order
async fn scan_window<Hook>(
    window: &[ParsedRecord],
    context: &ScanContext<'_, Hook>,
) -> Vec<(usize, WindowOutcome<Hook::Failure>)>
where
    Hook: GuardrailsOnlyPreCall,
{
    let answers = join_all(window.iter().map(|record| scan_record(record, context))).await;
    let mut outcomes = Vec::new();
    for (record, answer) in window.iter().zip(answers) {
        match answer {
            Ok(Some(outcome)) => {
                outcomes.push((record.line_number, WindowOutcome::Outcome(outcome)))
            }
            Ok(None) => (),
            Err(failure) => outcomes.push((record.line_number, WindowOutcome::Raised(failure))),
        }
    }
    outcomes
}
