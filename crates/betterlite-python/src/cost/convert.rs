use betterlite_core::cost::usage::ResponseKind;
use betterlite_logging::fields::usage::Usage;
use betterlite_models::billed_tokens::BilledTokenRates;
use betterlite_models::cost_breakdown::CostBreakdown;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

mod tests;

// One exact money amount as a JSON number: the lexeme keeps every digit LiteLLM's float would round.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct Amount(
    #[serde(with = "rust_decimal::serde::arbitrary_precision")] pub(crate) Decimal,
);

pub(crate) const fn optional(amount: Option<Amount>) -> Option<Decimal> {
    match amount {
        Some(inner) => Some(inner.0),
        None => None,
    }
}

pub(crate) const fn optional_unit(value: Option<i64>) -> Option<Option<i64>> {
    match value {
        Some(inner) => Some(Some(inner)),
        None => None,
    }
}

// The rates LiteLLM hands set_cost_breakdown, under the names its BilledTokenRates dataclass declares.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct RatesView {
    pub(crate) input_cost_per_token: Amount,
    pub(crate) output_cost_per_token: Amount,
    pub(crate) cache_read_input_token_cost: Amount,
    // ast-grep-ignore: no-long-names
    pub(crate) cache_read_input_audio_token_cost: Amount,
    // ast-grep-ignore: no-long-names
    pub(crate) cache_creation_input_token_cost: Amount,
    // ast-grep-ignore: no-long-names
    pub(crate) cache_creation_input_token_cost_above_1hr: Amount,
    // ast-grep-ignore: no-long-names
    pub(crate) output_cost_per_reasoning_token: Amount,
}

impl From<BilledTokenRates> for RatesView {
    fn from(rates: BilledTokenRates) -> Self {
        Self {
            input_cost_per_token: Amount(rates.input_cost_per_token),
            output_cost_per_token: Amount(rates.output_cost_per_token),
            cache_read_input_token_cost: Amount(rates.cache_read_input_token_cost),
            // ast-grep-ignore: no-long-names
            cache_read_input_audio_token_cost: Amount(rates.audio_read),
            // ast-grep-ignore: no-long-names
            cache_creation_input_token_cost: Amount(rates.cache_write),
            // ast-grep-ignore: no-long-names
            cache_creation_input_token_cost_above_1hr: Amount(rates.cache_write_1h),
            // ast-grep-ignore: no-long-names
            output_cost_per_reasoning_token: Amount(rates.reasoning),
        }
    }
}

// get_usage_object's input: the response kind and the usage object it carries, the usage already dumped to JSON.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct UsageRequest {
    pub(crate) kind: Option<ResponseKind>,
    pub(crate) usage: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct UsageAnswer<'answer> {
    pub(crate) usage: Option<UsageRecord<'answer>>,
}

// The logging Usage as LiteLLM's Usage model reads it back: its own fields plus any numeric extra.
#[derive(Debug, Serialize)]
pub(crate) struct UsageRecord<'record> {
    pub(crate) prompt_tokens: i64,
    pub(crate) completion_tokens: i64,
    pub(crate) total_tokens: i64,
    pub(crate) prompt_tokens_details: Option<&'record IndexMap<String, Json>>,
    pub(crate) completion_tokens_details: Option<&'record IndexMap<String, Json>>,
    pub(crate) server_tool_use: Option<&'record IndexMap<String, Json>>,
    pub(crate) cost: Option<Amount>,
    #[serde(flatten)]
    pub(crate) extras: &'record IndexMap<String, Json>,
}

impl<'record> From<&'record Usage> for UsageRecord<'record> {
    fn from(usage: &'record Usage) -> Self {
        Self {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            prompt_tokens_details: usage.prompt_tokens_details.as_ref(),
            completion_tokens_details: usage.completion_tokens_details.as_ref(),
            server_tool_use: usage.server_tool_use.as_ref(),
            cost: usage.cost.map(Amount),
            extras: &usage.extras,
        }
    }
}

// What completion_cost and response_cost answer: the charge, the breakdown LiteLLM stores and its rates.
#[derive(Debug, Serialize)]
pub(crate) struct CostAnswer<'answer> {
    pub(crate) cost: Amount,
    pub(crate) breakdown: Option<&'answer CostBreakdown>,
    pub(crate) billed_token_rates: Option<RatesView>,
}

// The one boundary every native method crosses: the text into the shape that method takes.
pub(crate) fn parse<'text, T>(text: &'text str) -> Result<T, ConvertError>
where
    T: Deserialize<'text>,
{
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(ConvertError::Json { source }),
    }
}

// ast-grep-ignore: no-one-line-functions
pub(crate) fn encode<T>(value: &T) -> Result<String, ConvertError>
where
    T: Serialize + ?Sized,
{
    serde_json::to_string(value).map_err(|source| ConvertError::Encode { source })
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConvertError {
    #[error("the native request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}
