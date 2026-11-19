#![cfg(test)]

use rstest::rstest;

use super::{beta, dispatch, failures, forwarding, handler, overrides, poll, stream};

#[derive(Clone, Copy)]
enum Scenario {
    RunPoll,
    RunStream,
    RunInstructionsTools,
    RunFailure,
    RunDispatch,
    InstructionsToolsOverrides,
    StreamEventHandler,
    AssistantsBetaDefault,
}

// One case per oracle file, each driving the run facade over the fake SDK transport.
#[rstest]
#[case::run_poll(Scenario::RunPoll)]
#[case::run_stream(Scenario::RunStream)]
#[case::run_instructions_tools(Scenario::RunInstructionsTools)]
#[case::run_failure(Scenario::RunFailure)]
#[case::run_dispatch(Scenario::RunDispatch)]
#[case::instructions_tools_overrides(Scenario::InstructionsToolsOverrides)]
#[case::stream_event_handler(Scenario::StreamEventHandler)]
#[case::assistants_beta_default(Scenario::AssistantsBetaDefault)]
#[tokio::test(start_paused = true)]
async fn assistants_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::RunPoll => poll::check().await,
        Scenario::RunStream => stream::check().await,
        Scenario::RunInstructionsTools => forwarding::check().await,
        Scenario::RunFailure => failures::check().await,
        Scenario::RunDispatch => dispatch::check().await,
        Scenario::InstructionsToolsOverrides => overrides::check().await,
        Scenario::StreamEventHandler => handler::check().await,
        Scenario::AssistantsBetaDefault => beta::check().await,
    }
}
