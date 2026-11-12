use rust_decimal::Decimal;

use crate::decisions::Json;
use crate::tool_fees::error::{RateField, ToolFeeError};
use crate::tool_fees::model_info::{Rate, ToolModelInfo};
use crate::tool_fees::ports::ToolPriceSource;
use crate::tool_fees::prices::ToolPrices;
use crate::tool_signals::conversions::{FileSearchParams, TokenCounts};
use crate::tool_signals::kwargs::{FileSearchTool, WebSearchOptions};

mod tests;

const AZURE: &str = "azure";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VectorStoreUsage {
    pub storage_gb: Option<Decimal>,
    pub days: Option<Decimal>,
}

// Only the exact low, medium and high strings select their key; any other or absent size uses medium.
pub fn web_search_rate(
    options: Option<&WebSearchOptions>,
    model_info: Option<&ToolModelInfo>,
) -> Result<Decimal, ToolFeeError> {
    let Some(info) = model_info else {
        return Ok(Decimal::ZERO);
    };
    let size = options.and_then(|selected| selected.fields.get("search_context_size"));
    match size {
        Some(Json::Text(text)) if text == "low" => info
            .search_context
            .low
            .or_zero(RateField::SearchContextSizeLow),
        Some(Json::Text(text)) if text == "high" => info
            .search_context
            .high
            .or_zero(RateField::SearchContextSizeHigh),
        _ => info
            .search_context
            .medium
            .or_zero(RateField::SearchContextSizeMedium),
    }
}

fn storage_days(params: FileSearchParams) -> Option<(Decimal, Decimal)> {
    let storage = params.storage_gb.filter(|value| !value.is_zero())?;
    let days = params.days.filter(|value| !value.is_zero())?;
    Some((storage, days))
}

// A present Azure storage key ends the model-specific branch even when the quantities are falsey.
fn model_file_search_fee(
    info: &ToolModelInfo,
    azure: bool,
    storage: Option<(Decimal, Decimal)>,
) -> Result<Option<Decimal>, ToolFeeError> {
    if azure && info.file_search_per_gb_per_day != Rate::Absent {
        let Some((gigabytes, days)) = storage else {
            return Ok(None);
        };
        let rate = info
            .file_search_per_gb_per_day
            .or_zero(RateField::FileSearchCostPerGbPerDay)?;
        let stored = gigabytes.checked_mul(days).ok_or(ToolFeeError::Overflow)?;
        return Ok(Some(
            stored.checked_mul(rate).ok_or(ToolFeeError::Overflow)?,
        ));
    }
    if info.file_search_per_1k_calls != Rate::Absent {
        // The model's per-1k figure is returned as is, without dividing by 1000 again.
        let rate = info
            .file_search_per_1k_calls
            .or_zero(RateField::FileSearchCostPer1kCalls)?;
        return Ok(Some(rate));
    }
    Ok(None)
}

pub fn file_search_fee(
    file_search: Option<&FileSearchTool>,
    provider: Option<&str>,
    model_info: Option<&ToolModelInfo>,
    params: FileSearchParams,
    prices: &ToolPrices,
) -> Result<Decimal, ToolFeeError> {
    if file_search.is_none() {
        return Ok(Decimal::ZERO);
    }
    let azure = provider == Some(AZURE);
    let storage = storage_days(params);
    if let Some(info) = model_info
        && let Some(fee) = model_file_search_fee(info, azure, storage)?
    {
        return Ok(fee);
    }
    if !azure {
        return Ok(prices.openai_search_per_1k);
    }
    let Some((gigabytes, days)) = storage else {
        return Ok(Decimal::ZERO);
    };
    gigabytes
        .checked_mul(days)
        .and_then(|stored| stored.checked_mul(prices.azure_search_per_gb_day))
        .ok_or(ToolFeeError::Overflow)
}

pub fn vector_store_fee(
    usage: VectorStoreUsage,
    provider: Option<&str>,
    model_info: Option<&ToolModelInfo>,
    prices: &ToolPrices,
) -> Result<Decimal, ToolFeeError> {
    let rate = match model_info {
        Some(info) if info.vector_store_per_gb_per_day != Rate::Absent => info
            .vector_store_per_gb_per_day
            .or_zero(RateField::VectorStoreCostPerGbPerDay)?,
        _ if provider == Some(AZURE) => prices.azure_vector_per_gb_day,
        _ => return Ok(Decimal::ZERO),
    };
    usage
        .storage_gb
        .unwrap_or_default()
        .checked_mul(usage.days.unwrap_or_default())
        .and_then(|stored| stored.checked_mul(rate))
        .ok_or(ToolFeeError::Overflow)
}

// A falsey token count adds nothing and never reads its rate.
fn token_fee(tokens: Option<i64>, rate: Rate, field: RateField) -> Result<Decimal, ToolFeeError> {
    let Some(count) = tokens.filter(|count| *count != 0) else {
        return Ok(Decimal::ZERO);
    };
    let per_thousand = rate.or_zero(field)?;
    Decimal::from(count)
        .checked_div(Decimal::ONE_THOUSAND)
        .and_then(|thousands| thousands.checked_mul(per_thousand))
        .ok_or(ToolFeeError::Overflow)
}

// Any truthy model rate selects both custom sides with a missing side free; two zero rates use the defaults.
pub fn computer_use_fee(
    tokens: TokenCounts,
    provider: Option<&str>,
    model_info: Option<&ToolModelInfo>,
    prices: &ToolPrices,
) -> Result<Decimal, ToolFeeError> {
    let billable = tokens.input_tokens.is_some_and(|count| count != 0)
        || tokens.output_tokens.is_some_and(|count| count != 0);
    if provider != Some(AZURE) || !billable {
        return Ok(Decimal::ZERO);
    }
    let (input_rate, output_rate) = match model_info {
        Some(info)
            if matches!(info.computer_input_per_1k, Rate::Value(rate) if !rate.is_zero())
                || matches!(info.computer_output_per_1k, Rate::Value(rate) if !rate.is_zero()) =>
        {
            (info.computer_input_per_1k, info.computer_output_per_1k)
        }
        _ => (
            Rate::Value(prices.azure_computer_input_per_1k),
            Rate::Value(prices.azure_computer_output_per_1k),
        ),
    };
    let input = token_fee(
        tokens.input_tokens,
        input_rate,
        RateField::ComputerInputPer1k,
    )?;
    let output = token_fee(
        tokens.output_tokens,
        output_rate,
        RateField::ComputerOutputPer1k,
    )?;
    input.checked_add(output).ok_or(ToolFeeError::Overflow)
}

// No hard-coded session price exists: a missing, null or failed container rate means free.
pub fn code_interpreter_fee(
    sessions: Option<i64>,
    provider: Option<&str>,
    model_info: Option<&ToolModelInfo>,
    source: &dyn ToolPriceSource,
) -> Result<Decimal, ToolFeeError> {
    let Some(count) = sessions.filter(|count| *count != 0) else {
        return Ok(Decimal::ZERO);
    };
    let rate = match (model_info, provider) {
        (Some(info), _) if info.code_interpreter_per_session != Rate::Absent => info
            .code_interpreter_per_session
            .or_zero(RateField::CodeInterpreterCostPerSession)?,
        (_, Some(name)) if !name.is_empty() => {
            match source.container_session_rate(name).unwrap_or_default() {
                Rate::Value(value) => value,
                Rate::Absent | Rate::Null => return Ok(Decimal::ZERO),
            }
        }
        _ => return Ok(Decimal::ZERO),
    };
    Decimal::from(count)
        .checked_mul(rate)
        .ok_or(ToolFeeError::Overflow)
}
