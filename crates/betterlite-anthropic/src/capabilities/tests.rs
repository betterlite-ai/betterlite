#![cfg(test)]

mod budget;
mod effort;
mod fixture;
mod minimal;
mod speed;
mod support;
mod supported;

use rstest::rstest;

#[derive(Clone, Copy, Debug)]
enum Scenario {
    MinimalThinkingFloor,
    MaxTokenBudgetCap,
    EffortValidation,
    SpeedPolicy,
    SupportedParameterList,
}

#[rstest]
#[case::minimal_thinking_floor(Scenario::MinimalThinkingFloor)]
#[case::max_token_budget_cap(Scenario::MaxTokenBudgetCap)]
#[case::effort_validation(Scenario::EffortValidation)]
#[case::speed_policy(Scenario::SpeedPolicy)]
#[case::supported_parameter_list(Scenario::SupportedParameterList)]
#[tokio::test]
async fn capabilities_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::MinimalThinkingFloor => minimal::minimal_thinking_floor().await,
        Scenario::MaxTokenBudgetCap => budget::max_token_budget_cap(),
        Scenario::EffortValidation => effort::effort_validation().await,
        Scenario::SpeedPolicy => speed::speed_policy(),
        Scenario::SupportedParameterList => supported::supported_parameter_list().await,
    }
}
