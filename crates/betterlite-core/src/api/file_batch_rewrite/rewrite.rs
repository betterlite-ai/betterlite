use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Seek};

use tokio_util::task::TaskTracker;

use super::error::RewriteError;
use super::rewind::{RewindOnDrop, settle_spool};
use super::spool::{OwnedSpool, Spool};
use crate::api::file_batch_guardrails::lines::{RawLine, iter_lines};
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
    file_source: &mut Source,
    result: &BatchScanResult<Spool>,
    tasks: &TaskTracker,
    output: Spool,
) -> Result<Spool, RewriteError>
where
    Source: BufRead + Seek + Send,
{
    let plan = RewritePlan::of(result);
    let mut guarded = RewindOnDrop {
        source: file_source,
        attempted: false,
    };
    let mut close_on_drop = OwnedSpool(Some(output.clone()));
    let copied = copy_records(&mut guarded.source, &plan, tasks, &output).await;
    let outcome = copied.map(|()| output);
    let rewritten = settle_spool(&mut guarded, &mut close_on_drop, outcome)?;
    rewritten.rewind(tasks).await?;
    Ok(rewritten)
}

async fn copy_records<Source>(
    source: &mut Source,
    plan: &RewritePlan<'_>,
    tasks: &TaskTracker,
    output: &Spool,
) -> Result<(), RewriteError>
where
    Source: BufRead,
{
    let mut wrote_any = false;
    for raw in iter_lines(&mut *source) {
        let RawLine { line_number, bytes } = raw?;
        if plan.dropped.contains(&line_number) {
            continue;
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
        wrote_any = true;
    }
    Ok(())
}
