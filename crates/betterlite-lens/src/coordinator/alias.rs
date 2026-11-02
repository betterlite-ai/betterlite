use crate::analysis::{AnalysisError, ReadContent, ReadFuture};
use crate::models::execution::{Execution, ExecutionContent, RunAssessment, Sample, TracePart};
use crate::models::finding::{Evidence, FindingDraft};
use crate::models::messages::JobResult;
use indexmap::IndexMap;
use std::sync::Arc;

pub(super) fn aliased_sample(sample: Sample, originals: &IndexMap<String, Execution>) -> Sample {
    let executions = originals
        .iter()
        .map(|(alias, execution)| Execution {
            id: alias.clone(),
            ..execution.clone()
        })
        .collect();
    Sample {
        executions,
        ..sample
    }
}

pub(super) fn aliased_read(
    read: ReadContent,
    originals: IndexMap<String, Execution>,
) -> ReadContent {
    let read_alias = move |identity: String, cursor: String, offset: i64| -> ReadFuture {
        let looked_up = originals.get(&identity).cloned();
        let inner_read = Arc::clone(&read);
        Box::pin(async move {
            let found = looked_up.ok_or_else(|| {
                AnalysisError::Validation(format!("unknown execution alias: {identity}"))
            })?;
            let page = inner_read(found.id.clone(), cursor, offset).await?;
            Ok(ExecutionContent {
                execution: Execution {
                    id: identity.clone(),
                    ..found
                },
                parts: page
                    .parts
                    .into_iter()
                    .map(|part| TracePart {
                        execution_id: identity.clone(),
                        ..part
                    })
                    .collect(),
                next_cursor: page.next_cursor,
                partial: page.partial,
            })
        })
    };
    Arc::new(read_alias)
}

pub(super) fn restore(result: JobResult, originals: &IndexMap<String, Execution>) -> JobResult {
    let assessments: Vec<RunAssessment> = result
        .assessments
        .into_iter()
        .map(|assessment| RunAssessment {
            execution_id: originals.get(&assessment.execution_id).map_or_else(
                || assessment.execution_id.clone(),
                |execution| execution.id.clone(),
            ),
            ..assessment
        })
        .collect();
    let findings: Vec<FindingDraft> = result
        .findings
        .into_iter()
        .map(|finding| FindingDraft {
            evidence: finding
                .evidence
                .into_iter()
                .map(|evidence| Evidence {
                    execution_id: originals.get(&evidence.execution_id).map_or_else(
                        || evidence.execution_id.clone(),
                        |execution| execution.id.clone(),
                    ),
                    ..evidence
                })
                .collect(),
            ..finding
        })
        .collect();
    JobResult {
        assessments,
        findings,
        ..result
    }
}
