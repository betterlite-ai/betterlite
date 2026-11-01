use betterlite_models::cost_usage::modality_projection::{
    CacheCreationTokenDetails, CachedTokenCounters, PromptModalityUsage,
};
use betterlite_models::cost_usage::{CompletionDetails, PromptDetails};
use betterlite_models::modality_cost::completion::parse_completion_details;
use betterlite_models::modality_cost::error::ModalityCostError;
use betterlite_models::modality_cost::input_cost::{InputCostRates, calculate_input_cost};
use betterlite_models::modality_cost::prompt::{
    PromptTokensDetailsResult, get_billable_input_tokens, parse_prompt_tokens_details,
};
use betterlite_models::token_rates::ModelInfo;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::HelperError;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "prompt")]
    Prompt(Box<PromptInput>),
    #[serde(rename = "completion")]
    Completion {
        completion_tokens: i64,
        #[serde(default)]
        text_tokens: Option<i64>,
        #[serde(default)]
        audio_tokens: Option<i64>,
        #[serde(default)]
        image_tokens: Option<i64>,
        #[serde(default)]
        video_tokens: Option<i64>,
        #[serde(default)]
        reasoning_tokens: Option<i64>,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PromptInput {
    prompt_tokens: i64,
    #[serde(default)]
    cached_tokens: Option<i64>,
    #[serde(default)]
    cache_write_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_tokens: Option<i64>,
    #[serde(default)]
    cache_creation_token_details: Option<CacheCreationTokenDetails>,
    #[serde(default)]
    cached_tokens_details: Option<CachedTokenCounters>,
    #[serde(default)]
    text_tokens: Option<i64>,
    #[serde(default)]
    audio_tokens: Option<i64>,
    #[serde(default)]
    image_tokens: Option<i64>,
    #[serde(default)]
    video_tokens: Option<i64>,
    #[serde(default)]
    character_count: Option<i64>,
    #[serde(default)]
    image_count: Option<i64>,
    #[serde(default)]
    video_length_seconds: Option<Decimal>,
    #[serde(default)]
    audio_length_seconds: Option<Decimal>,
    #[serde(default)]
    query_count: Option<i64>,
    model_info: ModelInfo,
    prompt_base_cost: Decimal,
    cache_read_cost: Decimal,
    cache_creation_cost: Decimal,
    cache_creation_cost_above_1hr: Decimal,
    #[serde(default)]
    service_tier: Option<String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        match input {
            Input::Prompt(fields) => run_prompt(fields),
            Input::Completion {
                completion_tokens,
                text_tokens,
                audio_tokens,
                image_tokens,
                video_tokens,
                reasoning_tokens,
            } => run_completion(
                *completion_tokens,
                &CompletionDetails {
                    text_tokens: *text_tokens,
                    audio_tokens: *audio_tokens,
                    image_tokens: *image_tokens,
                    video_tokens: *video_tokens,
                    reasoning_tokens: *reasoning_tokens,
                },
            ),
        }
    }
}

fn run_prompt(fields: &PromptInput) -> crate::helpers::HelperResult {
    let usage = PromptModalityUsage {
        base: PromptDetails {
            text_tokens: fields.text_tokens,
            audio_tokens: fields.audio_tokens,
            image_tokens: fields.image_tokens,
            video_tokens: fields.video_tokens,
            web_search_requests: None,
        },
        cached_tokens: fields.cached_tokens,
        cache_write_tokens: fields.cache_write_tokens,
        cache_creation_tokens: fields.cache_creation_tokens,
        cache_creation_token_details: fields.cache_creation_token_details,
        cached_tokens_details: fields.cached_tokens_details,
        character_count: fields.character_count,
        image_count: fields.image_count,
        video_length_seconds: fields.video_length_seconds,
        audio_length_seconds: fields.audio_length_seconds,
        query_count: fields.query_count,
    };
    let details =
        parse_prompt_tokens_details(&usage).map_err(|error| modality_cost_error(&error))?;
    let billable = get_billable_input_tokens(fields.prompt_tokens, &usage)
        .map_err(|error| modality_cost_error(&error))?;
    let rates = InputCostRates {
        prompt_base_cost: fields.prompt_base_cost,
        cache_read_cost: fields.cache_read_cost,
        cache_creation_cost: fields.cache_creation_cost,
        cache_creation_cost_above_1hr: fields.cache_creation_cost_above_1hr,
    };
    let cost = calculate_input_cost(
        &details,
        &fields.model_info,
        &rates,
        fields.service_tier.as_deref(),
    )
    .map_err(|error| modality_cost_error(&error))?;
    Ok(prompt_result(&details, billable, cost))
}

fn prompt_result(
    details: &PromptTokensDetailsResult,
    billable: i64,
    cost: Decimal,
) -> serde_json::Value {
    let ttl = details.cache_creation_token_details.map(|ttl| {
        json!({
            "ephemeral_5m_input_tokens": ttl.ephemeral_5m_input_tokens,
            "ephemeral_1h_input_tokens": ttl.ephemeral_1h_input_tokens,
        })
    });
    json!({
        "cache_hit_tokens": details.cache_hit_tokens,
        "cache_hit_audio_tokens": details.cache_hit_audio_tokens,
        "cache_creation_tokens": details.cache_creation_tokens,
        "cache_creation_token_details": ttl,
        "text_tokens": details.text_tokens,
        "audio_tokens": details.audio_tokens,
        "image_tokens": details.image_tokens,
        "video_tokens": details.video_tokens,
        "character_count": details.character_count,
        "image_count": details.image_count,
        "video_length_seconds": details.video_length_seconds.normalize().to_string(),
        "audio_length_seconds": details.audio_length_seconds.normalize().to_string(),
        "query_count": details.query_count,
        "billable_input_tokens": billable,
        "input_cost": cost.normalize().to_string(),
    })
}

fn run_completion(
    completion_tokens: i64,
    usage: &CompletionDetails,
) -> crate::helpers::HelperResult {
    let details = parse_completion_details(completion_tokens, usage)
        .map_err(|error| modality_cost_error(&error))?;
    Ok(json!({
        "audio_tokens": details.audio_tokens,
        "text_tokens": details.text_tokens,
        "reasoning_tokens": details.reasoning_tokens,
        "image_tokens": details.image_tokens,
        "video_tokens": details.video_tokens,
    }))
}

fn modality_cost_error(error: &ModalityCostError) -> Box<HelperError> {
    let class = match error {
        ModalityCostError::CacheAllocationOverflow => "CacheAllocationOverflow",
        ModalityCostError::ReasoningRemovalOverflow => "ReasoningRemovalOverflow",
        ModalityCostError::PromptCostOverflow => "PromptCostOverflow",
        ModalityCostError::CacheWritingCostOverflow => "CacheWritingCostOverflow",
        ModalityCostError::Rate(_) => "RateError",
        ModalityCostError::Component(_) => "ComponentError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
