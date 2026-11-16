use std::collections::BTreeMap;

use betterlite_models::{
    cost_usage::CostUsage,
    decisions::Json,
    token_cost::prompt_overlap::prompt_tokens_details,
    usage_sum::{CompletionTokensDetails, PromptTokensDetails, Usage as SummedUsage, usage_sum},
};
use rust_decimal::Decimal;

use super::decode::{DecodedBatchLine, decoded_lines};
use super::error::{BatchOutputUsageError, BatchResult};
use super::pricing::{LineCostInput, line_cost, usage_prompt_extras};
use super::usage::{response_body, response_model, response_was_successful, usage_for_line};
use super::{
    BatchOutputDiagnostic, BatchOutputUsage, BatchOutputUsageRequest, BatchOutputUsageResult,
    VertexRowStats,
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

#[derive(Default)]
struct Totals {
    usages: Vec<SummedUsage>,
    prompt_cost: Decimal,
    completion_cost: Decimal,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    cache_read: i64,
    cache_creation: i64,
    models: Vec<String>,
    successful: usize,
    failed: usize,
    diagnostics: Vec<BatchOutputDiagnostic>,
}

pub(super) fn aggregate_output(
    service: &BatchOutputUsage<'_>,
    request: &BatchOutputUsageRequest,
) -> BatchResult<BatchOutputUsageResult> {
    let lines = decoded_lines(&request.content, &service.limits)?;
    let mut state = Totals::default();
    for line in &lines {
        match line {
            DecodedBatchLine::Entry(entry) => state.entry(entry, request, service)?,
            DecodedBatchLine::Malformed { message } => {
                tracing::warn!(error = message, "skipping malformed batch output line");
                state.diagnostics.push(BatchOutputDiagnostic::MalformedLine);
            }
            DecodedBatchLine::NonObject { kind } => {
                tracing::warn!(kind = *kind, "skipping non-object batch output line");
                state
                    .diagnostics
                    .push(BatchOutputDiagnostic::NonObjectLine { kind });
            }
        }
    }
    let result = state.finish(request)?;
    tracing::debug!(
        cost = %result.cost, usage = ?result.usage, models = ?result.models,
        successful = result.successful_requests, failed = result.failed_requests, "batch output aggregate"
    );
    Ok(result)
}

impl Totals {
    fn entry(
        &mut self,
        entry: &Json,
        request: &BatchOutputUsageRequest,
        service: &BatchOutputUsage<'_>,
    ) -> BatchResult<()> {
        if !response_was_successful(entry, request.provider) {
            self.failed = self.failed.saturating_add(1);
            return Ok(());
        }
        match line_stats(entry, request, service) {
            Ok(stats) => self.add(stats),
            Err(error) => {
                let custom_id = custom_id(entry);
                tracing::warn!(
                    ?custom_id,
                    %error,
                    "batch output line could not be costed, so it is billed at $0 and the rest of the batch is still billed"
                );
                self.successful = self.successful.saturating_add(1);
                Ok(())
            }
        }
    }
    fn add(&mut self, stats: LineStats) -> BatchResult<()> {
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
    fn finish(mut self, request: &BatchOutputUsageRequest) -> BatchResult<BatchOutputUsageResult> {
        let combined = usage_sum(&self.usages)?;
        let reasoning_tokens = combined
            .completion_tokens_details
            .as_ref()
            .and_then(|details| details.reasoning_tokens)
            .unwrap_or(0);
        let usage = self.batch_usage(&combined, reasoning_tokens);
        if let Some(model_name) = request.model_name.as_deref() {
            self.models = vec![model_name.to_owned()];
        }
        self.result(usage)
    }

    fn result(self, usage: SummedUsage) -> BatchResult<BatchOutputUsageResult> {
        let cost = self
            .prompt_cost
            .checked_add(self.completion_cost)
            .ok_or(BatchOutputUsageError::Overflow { field: "cost" })?;
        Ok(BatchOutputUsageResult {
            cost,
            prompt_cost: self.prompt_cost,
            completion_cost: self.completion_cost,
            usage,
            models: self.models,
            successful_requests: self.successful,
            failed_requests: self.failed,
            diagnostics: self.diagnostics,
        })
    }

    // LiteLLM's Usage kwargs create a detail only while its counter is positive.
    fn batch_usage(&self, combined: &SummedUsage, reasoning_tokens: i64) -> SummedUsage {
        let text_tokens = combined.completion_tokens.saturating_sub(reasoning_tokens);
        SummedUsage {
            prompt_tokens: combined.prompt_tokens,
            completion_tokens: combined.completion_tokens,
            total_tokens: combined.total_tokens,
            prompt_tokens_details: self.prompt_cache_details(),
            completion_tokens_details: (reasoning_tokens != 0).then(|| CompletionTokensDetails {
                text_tokens: Some(text_tokens.max(0)),
                audio_tokens: None,
                image_tokens: None,
                video_tokens: None,
                reasoning_tokens: Some(reasoning_tokens),
                accepted_prediction_tokens: None,
                rejected_prediction_tokens: None,
            }),
            numeric_extras: BTreeMap::new(),
        }
    }

    fn prompt_cache_details(&self) -> Option<PromptTokensDetails> {
        if self.cache_read <= 0 && self.cache_creation <= 0 {
            return None;
        }
        let mut details = PromptTokensDetails::default();
        if self.cache_read > 0 {
            details.cached_tokens = Some(self.cache_read);
        }
        if self.cache_creation > 0 {
            details.cache_write_tokens = Some(self.cache_creation);
            details.cache_creation_tokens = Some(self.cache_creation);
        }
        Some(details)
    }

    fn add_vertex(&mut self, stats: Option<VertexRowStats>) -> BatchResult<()> {
        let Some(row_stats) = stats else {
            self.failed = self.failed.saturating_add(1);
            return Ok(());
        };
        self.successful = self.successful.saturating_add(1);
        self.prompt_cost = add_decimal(self.prompt_cost, row_stats.prompt_cost, "prompt_cost")?;
        self.completion_cost = add_decimal(
            self.completion_cost,
            row_stats.completion_cost,
            "completion_cost",
        )?;
        self.prompt_tokens = add_count(
            self.prompt_tokens,
            row_stats.usage.prompt_tokens,
            "prompt_tokens",
        )?;
        self.completion_tokens = add_count(
            self.completion_tokens,
            row_stats.usage.completion_tokens,
            "completion_tokens",
        )?;
        self.total_tokens = add_count(self.total_tokens, row_stats.total_tokens, "total_tokens")?;
        if let Some(model) = row_stats.model
            && !self.models.contains(&model)
        {
            self.models.push(model);
        }
        Ok(())
    }

    fn finish_vertex(mut self, model_name: Option<&str>) -> BatchResult<BatchOutputUsageResult> {
        if let Some(name) = model_name {
            self.models = vec![name.to_owned()];
        }
        let usage = SummedUsage {
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens: self.total_tokens,
            prompt_tokens_details: None,
            completion_tokens_details: None,
            numeric_extras: BTreeMap::new(),
        };
        let result = self.result(usage)?;
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

fn line_stats(
    entry: &Json,
    request: &BatchOutputUsageRequest,
    service: &BatchOutputUsage<'_>,
) -> BatchResult<LineStats> {
    let provider = request.provider;
    let body = response_body(entry, provider)?;
    let usage = usage_for_line(body, provider, service.providers)?;
    let model = response_model(body);
    let cost = line_cost(
        &LineCostInput {
            body,
            usage: &usage,
            provider,
            model_name: request.model_name.as_deref(),
            response_model: model.as_deref(),
            model_info: request.model_info.as_ref(),
        },
        service.lookup,
        service.resolution,
    )?;
    let details = prompt_tokens_details(&CostUsage::from(&usage), &usage_prompt_extras(&usage))?;
    Ok(LineStats {
        prompt_cost: cost.prompt_cost,
        completion_cost: cost.completion_cost,
        usage,
        cache_read: details.cache_hit_tokens,
        cache_creation: details.cache_creation_tokens,
        model,
    })
}
fn custom_id(entry: &Json) -> Option<&Json> {
    match entry {
        Json::Object(fields) => fields.get("custom_id"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn add_decimal(total: Decimal, value: Decimal, field: &'static str) -> BatchResult<Decimal> {
    total
        .checked_add(value)
        .ok_or(BatchOutputUsageError::Overflow { field })
}

fn add_count(total: i64, value: i64, field: &'static str) -> BatchResult<i64> {
    total
        .checked_add(value)
        .ok_or(BatchOutputUsageError::Overflow { field })
}
