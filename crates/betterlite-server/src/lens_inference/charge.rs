use betterlite_openai::chat::response::ChatResponse;
use rust_decimal::Decimal;

use crate::lens_inference::quote::Deployment;

mod tests;

// the custom per-token rates completion_cost prices a response with when its deployment carries them
pub struct CustomRates<'model> {
    pub model: &'model str,
    pub input_cost_per_token: Decimal,
    pub output_cost_per_token: Decimal,
}

// litellm.completion_cost against the provider's published prices, wired by B068
pub trait CostCalculator: Send + Sync {
    fn completion_cost(
        &self,
        response: &ChatResponse,
        custom: Option<&CustomRates<'_>>,
    ) -> Result<Decimal, CostError>;
}

#[derive(Debug, thiserror::Error)]
pub enum CostError {
    #[error("the completion cost could not be computed")]
    Unavailable,
}

// inference.completion_charge: custom rates price it, else the positive provider cost, else the estimate
pub fn completion_charge(
    deployments: &[Deployment],
    response: &ChatResponse,
    estimate: Decimal,
    calculator: &dyn CostCalculator,
) -> Result<Decimal, CostError> {
    if let [deployment] = deployments {
        let params = &deployment.litellm_params;
        if let (Some(input), Some(output)) =
            (params.input_cost_per_token, params.output_cost_per_token)
        {
            let custom = CustomRates {
                model: &params.model,
                input_cost_per_token: input,
                output_cost_per_token: output,
            };
            return calculator.completion_cost(response, Some(&custom));
        }
    }
    let actual = calculator.completion_cost(response, None)?;
    if actual > Decimal::ZERO {
        Ok(actual)
    } else {
        Ok(estimate)
    }
}
