use betterlite_models::cost_usage::CostUsage;
use betterlite_models::off_peak::schema::OffPeakBlock;
use betterlite_models::off_peak::schema::RateField;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_cost::cache_default::provider_cache_read_default;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::token_cost::regional::provider_geo_multiplier;
use betterlite_models::token_cost::{GenericCostRequest, generic_cost_per_token};
use betterlite_models::token_rates::ModelInfo;
use jiff::tz::TimeZoneDatabase;
use serde_json::json;

use crate::helpers::HelperError;

mod input;

use input::GenericInput;
pub(crate) use input::Input;

struct NullLookup;

impl PriceLookup for NullLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        match input {
            Input::Generic(fields) => run_generic(fields),
            Input::ProviderGeo {
                provider_specific_entry,
                inference_geo,
            } => run_provider_geo(provider_specific_entry.as_ref(), inference_geo.as_deref()),
            Input::FireworksCacheDefault(fields) => run_fireworks_cache_default(
                &fields.model_info,
                fields.custom_llm_provider.as_deref(),
                fields.off_peak_pricing.as_ref(),
                fields.off_peak_cache_read_present,
            ),
        }
    }
}

fn run_generic(fields: &GenericInput) -> crate::helpers::HelperResult {
    let database = TimeZoneDatabase::from_env();
    let usage = CostUsage::from(fields.usage.clone());
    let prompt_extras: Option<PromptExtras> = fields.prompt_extras.clone().map(PromptExtras::from);
    let request = GenericCostRequest {
        model: "oracle-model",
        custom_llm_provider: fields.custom_llm_provider.as_deref(),
        usage: &usage,
        prompt_extras: prompt_extras.as_ref(),
        supplied_model_info: fields.supplied_model_info.as_ref(),
        service_tier: fields.service_tier.as_deref(),
        data_residency: fields.data_residency.as_deref(),
        vertex_location: fields.vertex_location.as_deref(),
        moment: fields.moment,
        tiered_pricing: &fields.tiered_pricing,
        off_peak_pricing: fields.off_peak_pricing.as_ref(),
        off_peak_cache_read_present: fields.off_peak_cache_read_present,
    };
    let result = generic_cost_per_token(&request, &NullLookup, &database)
        .map_err(|error| token_cost_error(&error))?;
    Ok(json!({
        "prompt_cost": result.prompt_cost.normalize().to_string(),
        "completion_cost": result.completion_cost.normalize().to_string(),
    }))
}

fn run_provider_geo(
    provider_specific_entry: Option<&PriceRecord>,
    inference_geo: Option<&str>,
) -> crate::helpers::HelperResult {
    let multiplier = provider_geo_multiplier(provider_specific_entry, inference_geo)
        .map_err(|error| token_cost_error(&error))?;
    Ok(json!({ "multiplier": multiplier.normalize().to_string() }))
}

fn run_fireworks_cache_default(
    model_info: &ModelInfo,
    custom_llm_provider: Option<&str>,
    off_peak_pricing: Option<&OffPeakBlock>,
    off_peak_cache_read_present: bool,
) -> crate::helpers::HelperResult {
    let (updated_model_info, updated_off_peak) = provider_cache_read_default(
        model_info,
        custom_llm_provider,
        off_peak_pricing,
        off_peak_cache_read_present,
    )
    .map_err(|error| token_cost_error(&error))?;
    let cache_read_rate = updated_model_info
        .get("cache_read_input_token_cost")
        .and_then(scalar_to_value);
    let off_peak_cache_read_rate = updated_off_peak
        .and_then(|block| block.cache_read_input_token_cost)
        .and_then(|field| {
            let as_scalar = match field {
                RateField::Bool(flag) => Scalar::Bool(flag),
                RateField::Number(number) => Scalar::Number(number),
                RateField::Text(text) => Scalar::Text(text),
                RateField::Other(_) => return None,
            };
            scalar_to_value(&as_scalar)
        });
    Ok(json!({
        "cache_read_input_token_cost": cache_read_rate,
        "off_peak_cache_read_input_token_cost": off_peak_cache_read_rate,
    }))
}

fn scalar_to_value(value: &Scalar) -> Option<serde_json::Value> {
    match value {
        Scalar::Null => Some(serde_json::Value::Null),
        Scalar::Bool(flag) => Some(json!(flag)),
        Scalar::Number(number) => Some(json!(number)),
        Scalar::Text(text) => Some(json!(text)),
        Scalar::Array(_) | Scalar::Object(_) => None,
    }
}

fn token_cost_error(error: &TokenCostError) -> Box<HelperError> {
    let class = match error {
        TokenCostError::UnknownModel => "UnknownModel",
        TokenCostError::InvalidGeoMultiplier => "InvalidGeoMultiplier",
        TokenCostError::Overflow => "Overflow",
        TokenCostError::Rate(_) => "RateError",
        TokenCostError::Modality(_) => "ModalityError",
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
