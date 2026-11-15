use indexmap::IndexSet;
use rust_decimal::Decimal;

use super::super::decode::{DecodedBatchLine, decoded_lines};
use super::super::pricing::BatchCostPricer;
use super::super::{
    BatchOutputDiagnostic, BatchOutputUsage, BatchOutputUsageError, BatchOutputUsageResult,
    BatchUsageTotals, BatchVertexUsageRequest, VertexRowStats,
};
use super::{add_count, add_decimal};

pub(crate) fn aggregate_vertex(
    service: &BatchOutputUsage<'_>,
    request: &BatchVertexUsageRequest,
) -> Result<BatchOutputUsageResult, BatchOutputUsageError> {
    let lines = decoded_lines(&request.content, &service.limits)?;
    let pricer = BatchCostPricer {
        lookup: service.lookup,
        resolution: service.resolution,
    };
    let mut totals = VertexTotals::default();
    for line in &lines {
        match line {
            DecodedBatchLine::Malformed { message } => {
                tracing::warn!(error = %message, "skipping malformed batch output line");
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
                totals.add(stats)?;
            }
        }
    }
    totals.finish(request.model_name.as_deref())
}

#[derive(Default)]
struct VertexTotals {
    prompt_cost: Decimal,
    completion_cost: Decimal,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    models: IndexSet<String>,
    successful: usize,
    failed: usize,
    diagnostics: Vec<BatchOutputDiagnostic>,
}

impl VertexTotals {
    fn add(&mut self, stats: Option<VertexRowStats>) -> Result<(), BatchOutputUsageError> {
        let Some(VertexRowStats {
            usage,
            total_tokens,
            model,
            prompt_cost,
            completion_cost,
        }) = stats
        else {
            self.failed = self.failed.saturating_add(1);
            return Ok(());
        };
        self.successful = self.successful.saturating_add(1);
        self.prompt_cost = add_decimal(self.prompt_cost, prompt_cost, "prompt_cost")?;
        self.completion_cost =
            add_decimal(self.completion_cost, completion_cost, "completion_cost")?;
        self.prompt_tokens = add_count(self.prompt_tokens, usage.prompt_tokens, "prompt_tokens")?;
        self.completion_tokens = add_count(
            self.completion_tokens,
            usage.completion_tokens,
            "completion_tokens",
        )?;
        self.total_tokens = add_count(self.total_tokens, total_tokens, "total_tokens")?;
        if let Some(model) = model {
            self.models.insert(model);
        }
        Ok(())
    }

    fn finish(
        self,
        model_name: Option<&str>,
    ) -> Result<BatchOutputUsageResult, BatchOutputUsageError> {
        let cost = self
            .prompt_cost
            .checked_add(self.completion_cost)
            .ok_or(BatchOutputUsageError::Overflow { field: "cost" })?;
        let models = match model_name {
            Some(name) => vec![name.to_owned()],
            None => self.models.into_iter().collect(),
        };
        let result = BatchOutputUsageResult {
            cost,
            prompt_cost: self.prompt_cost,
            completion_cost: self.completion_cost,
            usage: BatchUsageTotals {
                prompt_tokens: self.prompt_tokens,
                completion_tokens: self.completion_tokens,
                total_tokens: self.total_tokens,
                reasoning_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
            models,
            successful_requests: self.successful,
            failed_requests: self.failed,
            diagnostics: self.diagnostics,
        };
        tracing::info!(
            "vertex_ai batch cost: cost={}, prompt={}, completion={}, total={}, successful={}, failed={}",
            result.cost,
            result.usage.prompt_tokens,
            result.usage.completion_tokens,
            result.usage.total_tokens,
            result.successful_requests,
            result.failed_requests
        );
        Ok(result)
    }
}
