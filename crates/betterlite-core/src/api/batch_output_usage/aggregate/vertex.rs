use super::super::decode::{DecodedBatchLine, decoded_lines};
use super::super::error::BatchResult;
use super::super::pricing::BatchCostPricer;
use super::super::{
    BatchOutputDiagnostic, BatchOutputUsage, BatchOutputUsageResult, BatchVertexUsageRequest,
};
use super::Totals;

pub(crate) fn aggregate_vertex(
    service: &BatchOutputUsage<'_>,
    request: &BatchVertexUsageRequest,
) -> BatchResult<BatchOutputUsageResult> {
    let lines = decoded_lines(&request.content, &service.limits)?;
    let pricer = BatchCostPricer {
        lookup: service.lookup,
        resolution: service.resolution,
    };
    let mut totals = Totals::default();
    for line in &lines {
        match line {
            DecodedBatchLine::Malformed { message } => {
                tracing::warn!(error = message, "skipping malformed batch output line");
                totals
                    .diagnostics
                    .push(BatchOutputDiagnostic::MalformedLine);
            }
            DecodedBatchLine::NonObject { kind } => {
                tracing::warn!(kind = *kind, "skipping non-object batch output line");
                totals
                    .diagnostics
                    .push(BatchOutputDiagnostic::NonObjectLine { kind: *kind });
            }
            DecodedBatchLine::Entry(row) => {
                let stats = service.providers.vertex_row_stats(
                    row,
                    request.model_name.as_deref(),
                    request.model_info.as_ref(),
                    &pricer,
                );
                totals.add_vertex(stats)?;
            }
        }
    }
    totals.finish_vertex(request.model_name.as_deref())
}
