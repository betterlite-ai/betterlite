use rust_decimal::Decimal;

use crate::lens_billing::completion::CompletionAnswer;
use crate::lens_inference::charge::{self, CostCalculator};
use crate::lens_inference::quote::Deployment;

use super::AnalyzeError;

// inference.analyze returns the first choice's content, or "{}" when the model sent none
pub(super) fn answer_content(answer: &CompletionAnswer) -> Result<String, AnalyzeError> {
    let choice = answer
        .response
        .choices
        .first()
        .ok_or(AnalyzeError::NoChoices)?;
    let content = choice
        .message
        .content
        .clone()
        .filter(|content| !content.is_empty())
        .unwrap_or_else(|| "{}".to_owned());
    Ok(content)
}

// the billed header wins when the pipeline sent one, else the provider cost or the estimate
pub(super) fn answer_cost(
    answer: &CompletionAnswer,
    deployments: &[Deployment],
    estimate: Decimal,
    costs: &dyn CostCalculator,
) -> Result<Decimal, AnalyzeError> {
    let Some(billed) = answer.billed_cost else {
        return charge::completion_charge(deployments, &answer.response, estimate, costs)
            .map_err(AnalyzeError::from);
    };
    Ok(billed)
}
