use rstest::rstest;

use super::{Case, Variable, assert_unrelated_key_rejected, assert_variable};

#[rstest]
#[case::loads_bedrock_min_thinking_budget_tokens(Case::Loads {
    name: "BEDROCK_MIN_THINKING_BUDGET_TOKENS",
    default: super::super::BEDROCK_MIN_THINKING_BUDGET,
    variable: Variable::BedrockMin,
})]
#[case::rejects_unrelated_key(Case::UnrelatedKey)]
fn anthropic_capability_catalog_config_cases(#[case] case: Case) {
    match case {
        Case::Loads {
            name,
            default,
            variable,
        } => assert_variable(name, default, variable),
        Case::UnrelatedKey => assert_unrelated_key_rejected(),
    }
}
