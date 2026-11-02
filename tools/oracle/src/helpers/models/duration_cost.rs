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
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "audio_cost_per_second")]
    AudioCostPerSecond {
        model: String,
        custom_llm_provider: Option<String>,
        duration: Decimal,
        catalog: Option<PriceRecord>,
    },
    #[serde(rename = "wall_clock_per_second")]
    WallClockPerSecond {
        model: String,
        custom_llm_provider: Option<String>,
        response_time_ms: Option<Decimal>,
        catalog: Option<PriceRecord>,
    },
    #[serde(rename = "video")]
    Video {
        calculator: VideoCalculator,
        model: String,
        duration_seconds: Decimal,
        custom_llm_provider: Option<String>,
        model_info: Option<PriceRecord>,
        video_resolution: Option<String>,
        raw_catalog: Option<PriceRecord>,
    },
    #[serde(rename = "character_metric")]
    CharacterMetric {
        key: String,
        record: PriceRecord,
        above_128k_tokens: Vec<Decimal>,
    },
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
        run_inner(input).map_err(duration_cost_error)
    }
}

fn duration_cost_error(error: DurationCostError) -> Box<crate::helpers::HelperError> {
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

fn run_inner(input: &Input) -> Result<serde_json::Value, DurationCostError> {
    match input {
        Input::AudioCostPerSecond {
            model,
            custom_llm_provider,
            duration,
            catalog,
        } => {
            let lookup = CaseLookup {
                effective: catalog.clone(),
                raw: None,
            };
            let (prompt_cost, completion_cost) =
                cost_per_second(model, custom_llm_provider.as_deref(), *duration, &lookup)?;
            Ok(serde_json::json!({
                "prompt_cost": prompt_cost.normalize().to_string(),
                "completion_cost": completion_cost.normalize().to_string(),
            }))
        }
        Input::WallClockPerSecond {
            model,
            custom_llm_provider,
            response_time_ms,
            catalog,
        } => {
            let lookup = CaseLookup {
                effective: catalog.clone(),
                raw: None,
            };
            let result = per_second_pricing_cost(
                model,
                custom_llm_provider.as_deref(),
                *response_time_ms,
                &lookup,
            )?;
            Ok(
                result.map_or(serde_json::Value::Null, |(prompt_cost, completion_cost)| {
                    serde_json::json!({
                        "prompt_cost": prompt_cost.normalize().to_string(),
                        "completion_cost": completion_cost.normalize().to_string(),
                    })
                }),
            )
        }
        Input::Video {
            calculator,
            model,
            duration_seconds,
            custom_llm_provider,
            model_info,
            video_resolution,
            raw_catalog,
        } => {
            let lookup = CaseLookup {
                effective: model_info.clone(),
                raw: raw_catalog.clone(),
            };
            let request = VideoCostRequest {
                model,
                duration_seconds: *duration_seconds,
                custom_llm_provider: custom_llm_provider.as_deref(),
                model_info: model_info.as_ref(),
                video_resolution: video_resolution.as_deref(),
            };
            let cost = match calculator {
                VideoCalculator::Openai => video_generation_cost(&request, &lookup)?,
                VideoCalculator::Default => default_video_cost_calculator(&request, &lookup)?,
            };
            Ok(serde_json::json!({ "cost": cost.normalize().to_string() }))
        }
        Input::CharacterMetric {
            key,
            record,
            above_128k_tokens,
        } => {
            let metric_name = match select_cost_metric_for_model(key, record)? {
                CostMetric::Character => "cost_per_character",
                CostMetric::Token => "cost_per_token",
            };
            let above_128k: Vec<bool> = above_128k_tokens
                .iter()
                .map(|tokens| *tokens > Decimal::from(128_000_i32))
                .collect();
            Ok(serde_json::json!({ "metric": metric_name, "above_128k": above_128k }))
        }
    }
}

mod tests;
