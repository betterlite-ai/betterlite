use std::collections::BTreeMap;

use betterlite_models::cost_usage::{CompletionDetails, CostUsage, PromptDetails};
use betterlite_openai::perplexity_cost::{PerplexityUsage, cost_per_token};
use betterlite_prompts::arguments::json::{Json, Object};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::Deserialize;
use serde_json::{Value, json};

use super::failure;
use crate::helpers::HelperResult;

const PRICE_SCALE: u32 = 12;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Steps {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    // the pinned cost function takes the model name, which the resolved model info already replaced
    #[serde(rename = "model")]
    _model: String,
    model_info: Object,
    usage: UsageInput,
    current_time: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct UsageInput {
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    citation_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    cost: Option<Json>,
    prompt_tokens_details: Option<PromptInput>,
    completion_tokens_details: Option<CompletionInput>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PromptInput {
    web_search_requests: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CompletionInput {
    reasoning_tokens: Option<i64>,
}

pub(super) fn run(steps: &Steps) -> HelperResult {
    let database = TimeZoneDatabase::from_env();
    let mut outcomes = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        let moment = DateTime::parse_from_rfc3339(&step.current_time)
            .map_err(failure)?
            .with_timezone(&Utc);
        let usage = usage_input(&step.usage);
        let result =
            cost_per_token(&step.model_info, &usage, moment, &database).map_err(failure)?;
        outcomes.push(json!({
            "prompt_cost": price_text(result.prompt_cost),
            "completion_cost": price_text(result.completion_cost),
        }));
    }
    Ok(Value::Array(outcomes))
}

fn usage_input(input: &UsageInput) -> PerplexityUsage {
    PerplexityUsage {
        tokens: CostUsage {
            prompt_tokens: input.prompt_tokens,
            completion_tokens: input.completion_tokens,
            total_tokens: input.total_tokens,
            prompt_details: input
                .prompt_tokens_details
                .as_ref()
                .map(|details| PromptDetails {
                    text_tokens: None,
                    audio_tokens: None,
                    image_tokens: None,
                    video_tokens: None,
                    web_search_requests: details.web_search_requests,
                }),
            completion_details: input.completion_tokens_details.as_ref().map(|details| {
                CompletionDetails {
                    text_tokens: None,
                    audio_tokens: None,
                    image_tokens: None,
                    video_tokens: None,
                    reasoning_tokens: details.reasoning_tokens,
                }
            }),
            cache_read_tokens: None,
            numeric_extras: BTreeMap::new(),
        },
        cost: input.cost.clone(),
        citation_tokens: input.citation_tokens,
        reasoning_tokens: input.reasoning_tokens,
    }
}

// costs compare as twelve-place decimals, since LiteLLM computes them in binary floats
fn price_text(value: Decimal) -> String {
    let rounded = value.round_dp_with_strategy(PRICE_SCALE, RoundingStrategy::MidpointNearestEven);
    format!("{rounded:.12}")
}
