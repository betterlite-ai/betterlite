use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Seek};

use tokio_util::task::TaskTracker;

use super::body::UploadBody;
use super::error::RewriteError;
use super::rewind::{RewindOnDrop, settle_spool};
use super::spool::{OwnedSpool, Spool};
use crate::api::file_batch_guardrails::SCAN_WINDOW;
use crate::api::file_batch_guardrails::error::BatchGuardrailError;
use crate::api::file_batch_guardrails::lines::RawLine;
use crate::api::file_batch_guardrails::records::{BatchScanResult, RecordChange, RecordRedacted};

// The lines the scan rewrote or dropped, and where each rewritten record waits in the scan's spool
struct RewritePlan<'result> {
    redacted: BTreeMap<usize, &'result RecordRedacted>,
    dropped: BTreeSet<usize>,
    spool: &'result Spool,
}

impl<'result> RewritePlan<'result> {
    fn of(result: &'result BatchScanResult<Spool>) -> Self {
        let mut redacted = BTreeMap::new();
        let mut dropped = BTreeSet::new();
        for change in &result.changes {
            match change {
                RecordChange::Redacted(record) => {
                    redacted.insert(record.line_number, record);
                }
                RecordChange::Dropped(record) => {
                    dropped.insert(record.line_number);
                }
            }
        }
        Self {
            redacted,
            dropped,
            spool: &result.redactions,
        }
    }
}

// Untouched records are copied through as written; rewrites come back from the spool; dropped records and blank lines are left out
pub async fn rewrite_batch_input_file<Source>(
    file_source: &UploadBody<Source>,
    result: &BatchScanResult<Spool>,
    tasks: &TaskTracker,
    output: Spool,
) -> Result<Spool, RewriteError>
where
    Source: BufRead + Seek + Send + 'static,
{
    let plan = RewritePlan::of(result);
    let mut guarded = RewindOnDrop {
        body: file_source,
        tasks,
        attempted: false,
    };
    let mut close_on_drop = OwnedSpool(Some(output.clone()));
    let copied = copy_records(file_source, &plan, tasks, &output).await;
    let outcome = copied.map(|()| output);
    let rewritten = settle_spool(&mut guarded, &mut close_on_drop, outcome).await?;
    rewritten.rewind(tasks).await?;
    Ok(rewritten)
}

async fn copy_records<Source>(
    body: &UploadBody<Source>,
    plan: &RewritePlan<'_>,
    tasks: &TaskTracker,
    output: &Spool,
) -> Result<(), RewriteError>
where
    Source: BufRead + Send + 'static,
{
    let mut wrote_any = false;
    let mut first = 1_usize;
    loop {
        let (lines, consumed) = body
            .window(
                tasks,
                first,
                SCAN_WINDOW,
                Ok::<RawLine, BatchGuardrailError>,
            )
            .await?;
        first = first.saturating_add(consumed);
        if lines.is_empty() {
            break;
        }
        let full = lines.len() == SCAN_WINDOW;
        for line in lines {
            wrote_any = emit_line(plan, tasks, output, wrote_any, line).await?;
        }
        if !full {
            break;
        }
    }
    Ok(())
}

// One scanned line becomes the bytes the rewrite writes: a spooled rewrite, or the line as it was minus its LF; a dropped line writes nothing and the separator stays with the written records
async fn emit_line(
    plan: &RewritePlan<'_>,
    tasks: &TaskTracker,
    output: &Spool,
    wrote_any: bool,
    line: RawLine,
) -> Result<bool, RewriteError> {
    let RawLine { line_number, bytes } = line;
    if plan.dropped.contains(&line_number) {
        return Ok(wrote_any);
    }
    let emitted = match plan.redacted.get(&line_number) {
        Some(change) => {
            plan.spool
                .read_at(tasks, change.offset, change.length)
                .await?
        }
        None => bytes
            .strip_suffix(b"\n")
            .unwrap_or(bytes.as_slice())
            .to_vec(),
    };
    let mut written = Vec::new();
    if wrote_any {
        written.push(b'\n');
    }
    written.extend_from_slice(&emitted);
    output.append(tasks, written).await?;
    Ok(true)
}
