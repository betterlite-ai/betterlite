use betterlite_models::duration_cost::character::{CostMetric, select_cost_metric_for_model};
use betterlite_models::duration_cost::error::DurationCostError;
use betterlite_models::duration_cost::openai_audio::{cost_per_second, video_generation_cost};
use betterlite_models::duration_cost::video::{VideoCostRequest, default_video_cost_calculator};
use betterlite_models::duration_cost::wall_clock::per_second_pricing_cost;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use rust_decimal::Decimal;
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VideoCalculator {
    Openai,
    Default,
}

#[derive(Deserialize)]
pub(crate) struct AudioCostPerSecondInput {
    model: String,
    custom_llm_provider: Option<String>,
    duration: Decimal,
    catalog: Option<PriceRecord>,
}

#[derive(Deserialize)]
pub(crate) struct WallClockPerSecondInput {
    model: String,
    custom_llm_provider: Option<String>,
    response_time_ms: Option<Decimal>,
    catalog: Option<PriceRecord>,
}

#[derive(Deserialize)]
pub(crate) struct VideoInput {
    calculator: VideoCalculator,
    model: String,
    duration_seconds: Decimal,
    custom_llm_provider: Option<String>,
    model_info: Option<PriceRecord>,
    video_resolution: Option<String>,
    raw_catalog: Option<PriceRecord>,
}

#[derive(Deserialize)]
pub(crate) struct CharacterMetricInput {
    key: String,
    record: PriceRecord,
    above_128k_tokens: Vec<Decimal>,
}

#[derive(Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "audio_cost_per_second")]
    AudioCostPerSecond(AudioCostPerSecondInput),
    #[serde(rename = "wall_clock_per_second")]
    WallClockPerSecond(WallClockPerSecondInput),
    #[serde(rename = "video")]
    Video(VideoInput),
    #[serde(rename = "character_metric")]
    CharacterMetric(CharacterMetricInput),
}

// Ignores the model and provider the lookup is asked about, same as the other case-driven PriceLookup fixtures in this helper group.
struct CaseLookup {
    effective: Option<PriceRecord>,
    raw: Option<PriceRecord>,
}

impl PriceLookup for CaseLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.effective.clone()
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        self.raw.clone()
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.effective.clone().map(|record| PricedModel {
            record,
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        run_inner(input).map_err(|error| duration_cost_error(&error))
    }
}

fn duration_cost_error(error: &DurationCostError) -> Box<crate::helpers::HelperError> {
    let class = match error {
        DurationCostError::UnknownModel { .. } => "UnknownModel",
        DurationCostError::NoCharacterOrTokenPricing { .. } => "NoCharacterOrTokenPricing",
        DurationCostError::Overflow => "Overflow",
        DurationCostError::Price(_) => "Price",
        DurationCostError::TokenCost(_) => "TokenCost",
    };
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn catalog_lookup(catalog: Option<&PriceRecord>) -> CaseLookup {
    CaseLookup {
        effective: catalog.cloned(),
        raw: None,
    }
}

fn cost_pair_json(prompt_cost: Decimal, completion_cost: Decimal) -> serde_json::Value {
    serde_json::json!({
        "prompt_cost": prompt_cost.normalize().to_string(),
        "completion_cost": completion_cost.normalize().to_string(),
    })
}

fn wall_clock_json(result: Option<(Decimal, Decimal)>) -> serde_json::Value {
    match result {
        Some((prompt_cost, completion_cost)) => serde_json::json!({
            "charged": true,
            "prompt_cost": prompt_cost.normalize().to_string(),
            "completion_cost": completion_cost.normalize().to_string(),
        }),
        None => serde_json::json!({ "charged": false }),
    }
}

fn run_inner(input: &Input) -> Result<serde_json::Value, DurationCostError> {
    match input {
        Input::AudioCostPerSecond(fields) => {
            let lookup = catalog_lookup(fields.catalog.as_ref());
            let (prompt_cost, completion_cost) = cost_per_second(
                &fields.model,
                fields.custom_llm_provider.as_deref(),
                fields.duration,
                &lookup,
            )?;
            Ok(cost_pair_json(prompt_cost, completion_cost))
        }
        Input::WallClockPerSecond(fields) => {
            let lookup = catalog_lookup(fields.catalog.as_ref());
            let result = per_second_pricing_cost(
                &fields.model,
                fields.custom_llm_provider.as_deref(),
                fields.response_time_ms,
                &lookup,
            )?;
            Ok(wall_clock_json(result))
        }
        Input::Video(fields) => run_video(fields),
        Input::CharacterMetric(fields) => {
            let metric_name = match select_cost_metric_for_model(&fields.key, &fields.record)? {
                CostMetric::Character => "cost_per_character",
                CostMetric::Token => "cost_per_token",
            };
            let above_128k: Vec<bool> = fields
                .above_128k_tokens
                .iter()
                .map(|tokens| *tokens > Decimal::from(128_000_i32))
                .collect();
            Ok(serde_json::json!({ "metric": metric_name, "above_128k": above_128k }))
        }
    }
}

fn run_video(fields: &VideoInput) -> Result<serde_json::Value, DurationCostError> {
    let lookup = CaseLookup {
        effective: fields.model_info.clone(),
        raw: fields.raw_catalog.clone(),
    };
    let request = VideoCostRequest {
        model: &fields.model,
        duration_seconds: fields.duration_seconds,
        custom_llm_provider: fields.custom_llm_provider.as_deref(),
        model_info: fields.model_info.as_ref(),
        video_resolution: fields.video_resolution.as_deref(),
    };
    let cost = match fields.calculator {
        VideoCalculator::Openai => video_generation_cost(&request, &lookup),
        VideoCalculator::Default => default_video_cost_calculator(&request, &lookup),
    }?;
    Ok(serde_json::json!({ "cost": cost.normalize().to_string() }))
}

mod tests;
