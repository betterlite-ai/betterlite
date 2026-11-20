use std::collections::BTreeMap;

use rust_decimal::Decimal;

use super::decode::{decode_model_usage_fields, usage_date};
use super::{
    ModelUsageError, ModelUsageKey, ModelUsagePayload, ModelUsageRollup, ModelUsageTransaction,
};

const UNKNOWN: &str = "unknown";
const SUCCESS_STATUS: &str = "success";

// build_model_usage_transaction: a payload without a date or with an internal call origin rolls up nothing, and every missing identity reads as the source's fallback.
#[must_use]
pub fn build_model_usage_transaction(payload: &ModelUsagePayload) -> Option<ModelUsageTransaction> {
    let date = usage_date(&payload.start_time)?;
    let fields =
        decode_model_usage_fields(payload.metadata.as_bytes(), payload.request_tags.as_bytes());
    if fields.internal_call {
        return None;
    }
    let model = payload.model.clone().unwrap_or_else(|| UNKNOWN.to_owned());
    let model_group = payload.model_group.clone().unwrap_or_else(|| model.clone());
    let custom_llm_provider = payload
        .custom_llm_provider
        .clone()
        .unwrap_or_else(|| UNKNOWN.to_owned());
    Some(ModelUsageTransaction {
        key: ModelUsageKey {
            date,
            model_group,
            model,
            custom_llm_provider,
            task_type: fields.task_type,
        },
        spend: payload.spend,
        prompt_tokens: payload.prompt_tokens,
        completion_tokens: payload.completion_tokens,
        successful: payload.status == SUCCESS_STATUS,
    })
}

// flush_model_usage_transactions' grouping: one row per key in date, group, model, provider and task order, with additive totals and a success/failure split.
pub fn model_usage_rollups(
    transactions: &[ModelUsageTransaction],
) -> Result<Vec<ModelUsageRollup>, ModelUsageError> {
    let mut grouped: BTreeMap<&ModelUsageKey, ModelUsageRollup> = BTreeMap::new();
    for transaction in transactions {
        let total = grouped
            .entry(&transaction.key)
            .or_insert_with(|| ModelUsageRollup {
                key: transaction.key.clone(),
                spend: Decimal::ZERO,
                prompt_tokens: 0,
                completion_tokens: 0,
                request_count: 0,
                successful_requests: 0,
                failed_requests: 0,
            });
        total.accumulate(transaction)?;
    }
    let mut rollups: Vec<ModelUsageRollup> = grouped.into_values().collect();
    for rollup in &mut rollups {
        rollup.failed_requests = rollup
            .request_count
            .saturating_sub(rollup.successful_requests);
    }
    Ok(rollups)
}

impl ModelUsageRollup {
    // accumulate: one entry's totals and its success or failure, checked so an overflow reports instead of wrapping.
    fn accumulate(&mut self, transaction: &ModelUsageTransaction) -> Result<(), ModelUsageError> {
        self.spend = self
            .spend
            .checked_add(transaction.spend.0)
            .ok_or(ModelUsageError::SpendOverflow)?;
        self.prompt_tokens = self
            .prompt_tokens
            .checked_add(transaction.prompt_tokens)
            .ok_or(ModelUsageError::CountOverflow)?;
        self.completion_tokens = self
            .completion_tokens
            .checked_add(transaction.completion_tokens)
            .ok_or(ModelUsageError::CountOverflow)?;
        self.request_count = self
            .request_count
            .checked_add(1)
            .ok_or(ModelUsageError::CountOverflow)?;
        if transaction.successful {
            self.successful_requests = self
                .successful_requests
                .checked_add(1)
                .ok_or(ModelUsageError::CountOverflow)?;
        }
        Ok(())
    }
}
