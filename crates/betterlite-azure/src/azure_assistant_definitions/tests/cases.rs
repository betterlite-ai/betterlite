#![cfg(test)]

use rstest::rstest;

use super::{beta, create, delete, flags, inputs, list};

#[derive(Clone, Copy)]
enum Scenario {
    ListAssistants,
    CreateAssistant,
    DeleteAssistant,
    AssistantsBetaDefault,
    AzureAssistantClientInputs,
    AzureAssistantAsyncFlags,
}

// One case per oracle file, each driving the production operation over the fake SDK transport.
#[rstest]
#[case::list_assistants(Scenario::ListAssistants)]
#[case::create_assistant(Scenario::CreateAssistant)]
#[case::delete_assistant(Scenario::DeleteAssistant)]
#[case::assistants_beta_default(Scenario::AssistantsBetaDefault)]
#[case::azure_assistant_client_inputs(Scenario::AzureAssistantClientInputs)]
#[case::azure_assistant_async_flags(Scenario::AzureAssistantAsyncFlags)]
#[tokio::test]
async fn azure_assistant_definitions_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::ListAssistants => list::check().await,
        Scenario::CreateAssistant => create::check().await,
        Scenario::DeleteAssistant => delete::check().await,
        Scenario::AssistantsBetaDefault => beta::check().await,
        Scenario::AzureAssistantClientInputs => inputs::check().await,
        Scenario::AzureAssistantAsyncFlags => flags::check().await,
    }
}
