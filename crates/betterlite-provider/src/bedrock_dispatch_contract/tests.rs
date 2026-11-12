#![cfg(test)]

use rstest::rstest;

mod plan_cases;
mod unsupported_operation;

#[derive(Debug, Clone, Copy)]
enum Scenario {
    InvokePlan,
    ConversePlan,
    OperationErrorPropagation,
}

#[rstest]
#[case::invoke_plan(Scenario::InvokePlan)]
#[case::converse_plan(Scenario::ConversePlan)]
#[case::operation_error_propagation(Scenario::OperationErrorPropagation)]
fn bedrock_dispatch_contract_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::InvokePlan => plan_cases::invoke_plan(),
        Scenario::ConversePlan => plan_cases::converse_plan(),
        Scenario::OperationErrorPropagation => plan_cases::operation_error_propagation(),
    }
}
