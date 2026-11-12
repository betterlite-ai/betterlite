#![cfg(test)]

mod fixture;
mod scenarios;

use rstest::rstest;

#[derive(Clone, Copy, Debug)]
enum Scenario {
    ProviderQualifiedCatalog,
    AdaptiveThinking,
    ForcedToolDowngrade,
    SamplingCapability,
    LegacyBudgetEffort,
    DisabledThinkingWarning,
    EffortBetaGate,
}

#[rstest]
#[case::provider_qualified_catalog(Scenario::ProviderQualifiedCatalog)]
#[case::adaptive_thinking(Scenario::AdaptiveThinking)]
#[case::forced_tool_downgrade(Scenario::ForcedToolDowngrade)]
#[case::sampling_capability(Scenario::SamplingCapability)]
#[case::legacy_budget_effort(Scenario::LegacyBudgetEffort)]
#[case::disabled_thinking_warning(Scenario::DisabledThinkingWarning)]
#[case::effort_beta_gate(Scenario::EffortBetaGate)]
#[tokio::test]
async fn anthropic_capability_catalog_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::ProviderQualifiedCatalog => scenarios::provider_qualified_catalog().await,
        Scenario::AdaptiveThinking => scenarios::adaptive_thinking().await,
        Scenario::ForcedToolDowngrade => scenarios::forced_tool_downgrade(),
        Scenario::SamplingCapability => scenarios::sampling_capability(),
        Scenario::LegacyBudgetEffort => scenarios::legacy_budget_effort().await,
        Scenario::DisabledThinkingWarning => scenarios::disabled_thinking_warning().await,
        Scenario::EffortBetaGate => scenarios::effort_beta_gate().await,
    }
}
