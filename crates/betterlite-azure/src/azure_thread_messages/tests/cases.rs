#![cfg(test)]

use rstest::rstest;

use super::{beta, create, flags, retrieve};

#[derive(Clone, Copy)]
enum Scenario {
    ThreadCreate,
    ThreadRetrieve,
    AssistantsBetaDefault,
    AzureThreadAsyncFlags,
}

// One case per oracle file, each driving the production operation over the fake SDK transport.
#[rstest]
#[case::thread_create(Scenario::ThreadCreate)]
#[case::thread_retrieve(Scenario::ThreadRetrieve)]
#[case::assistants_beta_default(Scenario::AssistantsBetaDefault)]
#[case::azure_thread_async_flags(Scenario::AzureThreadAsyncFlags)]
#[tokio::test]
async fn azure_thread_messages_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::ThreadCreate => create::check().await,
        Scenario::ThreadRetrieve => retrieve::check().await,
        Scenario::AssistantsBetaDefault => beta::check().await,
        Scenario::AzureThreadAsyncFlags => flags::check().await,
    }
}
