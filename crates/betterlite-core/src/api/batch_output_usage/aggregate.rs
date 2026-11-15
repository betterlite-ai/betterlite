use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::{Usage as SummedUsage, usage_sum};
use rust_decimal::Decimal;

use super::decode::{DecodedBatchLine, decoded_lines};
use super::error::{BatchLineError, BatchOutputUsageError};
use super::pricing::{LineCostInput, cache_counters, line_cost};
use super::usage::{response_body, response_model, response_was_successful, usage_for_line};
use super::{
    BatchOutputDiagnostic, BatchOutputUsage, BatchOutputUsageRequest, BatchOutputUsageResult,
    BatchUsageTotals,
};

pub(super) mod vertex;

struct LineStats {
    prompt_cost: Decimal,
    completion_cost: Decimal,
    usage: SummedUsage,
    cache_read: i64,
    cache_creation: i64,
    model: Option<String>,
}

pub(super) fn aggregate_output(
    service: &BatchOutputUsage<'_>,
    request: &BatchOutputUsageRequest,
) -> Result<BatchOutputUsageResult, BatchOutputUsageError> {
    let lines = decoded_lines(&request.content, &service.limits)?;
    let mut state = OutputState::new(request);
    for line in &lines {
        state.skip(line);
    }
    for line in &lines {
        if let DecodedBatchLine::Entry(entry) = line {
            state.entry(entry, service)?;
        }
    }
    let result = state.finish()?;
    tracing::debug!(
        cost = %result.cost,
        usage = ?result.usage,
        models = ?result.models,
        successful = result.successful_requests,
        failed = result.failed_requests,
        "batch output aggregate"
    );
    Ok(result)
}

struct OutputState<'request> {
    request: &'request BatchOutputUsageRequest,
    usages: Vec<SummedUsage>,
    prompt_cost: Decimal,
    completion_cost: Decimal,
    cache_read: i64,
    cache_creation: i64,
    models: Vec<String>,
    successful: usize,
    failed: usize,
    diagnostics: Vec<BatchOutputDiagnostic>,
}

impl<'request> OutputState<'request> {
    fn new(request: &'request BatchOutputUsageRequest) -> Self {
        Self {
            request,
            usages: Vec::new(),
            prompt_cost: Decimal::ZERO,
            completion_cost: Decimal::ZERO,
            cache_read: 0,
            cache_creation: 0,
            models: Vec::new(),
            successful: 0,
            failed: 0,
            diagnostics: Vec::new(),
        }
    }

    fn skip(&mut self, line: &DecodedBatchLine) {
        match line {
            DecodedBatchLine::Malformed { message } => {
                tracing::warn!(error = %message, "skipping malformed batch output line");
                self.diagnostics.push(BatchOutputDiagnostic::MalformedLine);
            }
            DecodedBatchLine::NonObject { kind } => {
                tracing::warn!(kind = *kind, "skipping non-object batch output line");
                self.diagnostics
                    .push(BatchOutputDiagnostic::NonObjectLine { kind: *kind });
            }
            DecodedBatchLine::Entry(_) => {}
        }
    }

    fn entry(
        &mut self,
        entry: &Json,
        service: &BatchOutputUsage<'_>,
    ) -> Result<(), BatchOutputUsageError> {
        if !response_was_successful(entry, self.request.provider) {
            self.failed = self.failed.saturating_add(1);
            return Ok(());
        }
        match self.compute(entry, service) {
            Ok(stats) => self.add(stats),
            Err(error) => {
                let custom_id = custom_id(entry);
                tracing::warn!(
                    ?custom_id,
                    error = %error,
                    "batch output line could not be costed, so it is billed at $0 and the rest of the batch is still billed"
                );
                self.diagnostics
                    .push(BatchOutputDiagnostic::UncostableLine { custom_id });
                self.successful = self.successful.saturating_add(1);
                Ok(())
            }
        }
    }

    fn compute(
        &mut self,
        entry: &Json,
        service: &BatchOutputUsage<'_>,
    ) -> Result<LineStats, BatchLineError> {
        let provider = self.request.provider;
        let body = response_body(entry, provider)?;
        let line_usage = usage_for_line(body, provider, service.providers)?;
        if let Some(diagnostic) = line_usage.unparsed {
            self.diagnostics.push(diagnostic);
        }
        let model = response_model(body);
        let cost = line_cost(
            &LineCostInput {
                body,
                usage: &line_usage.usage,
                provider,
                model_name: self.request.model_name.as_deref(),
                response_model: model.as_deref(),
                model_info: self.request.model_info.as_ref(),
            },
            service.lookup,
            service.resolution,
        )?;
        let (cache_read, cache_creation) = cache_counters(&line_usage.usage)?;
        Ok(LineStats {
            prompt_cost: cost.prompt_cost,
            completion_cost: cost.completion_cost,
            usage: line_usage.usage,
            cache_read,
            cache_creation,
            model,
        })
    }

    fn add(&mut self, stats: LineStats) -> Result<(), BatchOutputUsageError> {
        self.prompt_cost = add_decimal(self.prompt_cost, stats.prompt_cost, "prompt_cost")?;
        self.completion_cost = add_decimal(
            self.completion_cost,
            stats.completion_cost,
            "completion_cost",
        )?;
        self.cache_read = add_count(self.cache_read, stats.cache_read, "cache_read_tokens")?;
        self.cache_creation = add_count(
            self.cache_creation,
            stats.cache_creation,
            "cache_creation_tokens",
        )?;
        if let Some(model) = stats.model {
            self.models.push(model);
        }
        self.usages.push(stats.usage);
        self.successful = self.successful.saturating_add(1);
        Ok(())
    }

    fn finish(self) -> Result<BatchOutputUsageResult, BatchOutputUsageError> {
        let combined = usage_sum(&self.usages)?;
        let reasoning_tokens = combined
            .completion_tokens_details
            .as_ref()
            .and_then(|details| details.reasoning_tokens)
            .unwrap_or(0);
        let models = match self.request.model_name.as_deref() {
            Some(model_name) => vec![model_name.to_owned()],
            None => self.models,
        };
        let cost = self
            .prompt_cost
            .checked_add(self.completion_cost)
            .ok_or(BatchOutputUsageError::Overflow { field: "cost" })?;
        let usage = BatchUsageTotals {
            prompt_tokens: combined.prompt_tokens,
            completion_tokens: combined.completion_tokens,
            total_tokens: combined.total_tokens,
            reasoning_tokens,
            cache_read_tokens: self.cache_read,
            cache_creation_tokens: self.cache_creation,
        };
        Ok(BatchOutputUsageResult {
            cost,
            prompt_cost: self.prompt_cost,
            completion_cost: self.completion_cost,
            usage,
            models,
            successful_requests: self.successful,
            failed_requests: self.failed,
            diagnostics: self.diagnostics,
        })
    }
}

fn custom_id(entry: &Json) -> Option<Json> {
    let Json::Object(fields) = entry else {
        return None;
    };
    fields.get("custom_id").cloned()
}

fn add_decimal(
    total: Decimal,
    value: Decimal,
    field: &'static str,
) -> Result<Decimal, BatchOutputUsageError> {
    total
        .checked_add(value)
        .ok_or(BatchOutputUsageError::Overflow { field })
}

fn add_count(total: i64, value: i64, field: &'static str) -> Result<i64, BatchOutputUsageError> {
    total
        .checked_add(value)
        .ok_or(BatchOutputUsageError::Overflow { field })
}
