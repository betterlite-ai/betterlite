use betterlite_models::token_rates::{self, EffectiveRates, ModelInfo, Usage};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "base")]
    Base {
        model_info: ModelInfo,
        usage: Usage,
        service_tier: Option<String>,
        provider: Option<String>,
    },
    #[serde(rename = "batch")]
    Batch {
        model_info: ModelInfo,
        usage: Usage,
        provider: Option<String>,
    },
    #[serde(rename = "component")]
    Component {
        model_info: ModelInfo,
        cost_key: String,
        usage: Option<Decimal>,
    },
    #[serde(rename = "deployment")]
    Deployment { model_info: Option<ModelInfo> },
}

fn helper_error(error: impl std::fmt::Display) -> Box<crate::helpers::HelperError> {
    let result = Box::new(crate::helpers::HelperError {
        class: "TokenRateError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    });
    result
}

fn output_rates(rates: EffectiveRates) -> Value {
    let output = serde_json::json!({
        "input_rate": rates.input_rate.normalize().to_string(),
        "output_rate": rates.output_rate.normalize().to_string(),
        "cache_read_rate": rates.cache_read_rate.normalize().to_string(),
        "cache_creation_rate": rates.cache_creation_rate.normalize().to_string(),
        "cache_creation_above_1hr_rate": rates.cache_creation_above_1hr_rate.normalize().to_string(),
        "reasoning_rate": rates.reasoning_rate.normalize().to_string(),
    });
    output
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::Base {
                model_info,
                usage,
                service_tier,
                provider,
            } => output_rates(
                token_rates::get_token_base_cost(
                    model_info,
                    *usage,
                    service_tier.as_deref(),
                    provider.as_deref(),
                )
                .map_err(helper_error)?,
            ),
            Input::Batch {
                model_info,
                usage,
                provider,
            } => serde_json::to_value(
                token_rates::get_batch_cost_rates(model_info, *usage, provider.as_deref())
                    .map_err(helper_error)?,
            )
            .map_err(helper_error)?,
            Input::Component {
                model_info,
                cost_key,
                usage,
            } => serde_json::to_value(
                token_rates::calculate_cost_component(model_info, cost_key, *usage)
                    .map_err(helper_error)?
                    .normalize()
                    .to_string(),
            )
            .map_err(helper_error)?,
            Input::Deployment { model_info } => serde_json::to_value(
                token_rates::deployment_pricing(model_info.as_ref()).map_err(helper_error)?,
            )
            .map_err(helper_error)?,
        };
        Ok(output)
    }
}
