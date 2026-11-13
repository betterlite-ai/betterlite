#![cfg(test)]

mod fixture;
mod scenarios;

use rstest::rstest;

#[derive(Clone, Copy, Debug)]
enum Scenario {
    OneShotSubagentBilling,
    MalformedBillingText,
    NativeCompactionSignature,
    CacheControlSupport,
    FileAndPdfFlags,
    ToolSearchAndExamples,
    EagerInputBoolean,
    ComputerToolBeta,
    SkillsCodeExecutionBeta,
    ConversationFeatureFlags,
    CommonBetaSet,
    NativeWebSearchType,
}

#[rstest]
#[case::one_shot_subagent_billing(Scenario::OneShotSubagentBilling)]
#[case::malformed_billing_text(Scenario::MalformedBillingText)]
#[case::native_compaction_signature(Scenario::NativeCompactionSignature)]
#[case::cache_control_support(Scenario::CacheControlSupport)]
#[case::file_and_pdf_flags(Scenario::FileAndPdfFlags)]
#[case::tool_search_and_examples(Scenario::ToolSearchAndExamples)]
#[case::eager_input_boolean(Scenario::EagerInputBoolean)]
#[case::computer_tool_beta(Scenario::ComputerToolBeta)]
#[case::skills_code_execution_beta(Scenario::SkillsCodeExecutionBeta)]
#[case::conversation_feature_flags(Scenario::ConversationFeatureFlags)]
#[case::common_beta_set(Scenario::CommonBetaSet)]
#[case::native_web_search_type(Scenario::NativeWebSearchType)]
#[tokio::test]
async fn feature_flags_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::OneShotSubagentBilling => scenarios::one_shot_subagent_billing(),
        Scenario::MalformedBillingText => scenarios::malformed_billing_text(),
        Scenario::NativeCompactionSignature => scenarios::native_compaction_signature(),
        Scenario::CacheControlSupport => scenarios::cache_control_support().await,
        Scenario::FileAndPdfFlags => scenarios::file_and_pdf_flags(),
        Scenario::ToolSearchAndExamples => scenarios::tool_search_and_examples(),
        Scenario::EagerInputBoolean => scenarios::eager_input_boolean(),
        Scenario::ComputerToolBeta => scenarios::computer_tool_beta(),
        Scenario::SkillsCodeExecutionBeta => scenarios::skills_code_execution_beta(),
        Scenario::ConversationFeatureFlags => scenarios::conversation_feature_flags(),
        Scenario::CommonBetaSet => scenarios::common_beta_set().await,
        Scenario::NativeWebSearchType => scenarios::native_web_search_type(),
    }
}
