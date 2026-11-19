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
#[case::case_1_run_poll(Scenario::RunPoll)]
#[case::case_2_run_stream(Scenario::RunStream)]
#[case::case_3_run_instructions_tools(Scenario::RunInstructionsTools)]
#[case::case_4_run_failure(Scenario::RunFailure)]
#[case::case_5_run_dispatch(Scenario::RunDispatch)]
#[case::case_6_instructions_tools_overrides(Scenario::InstructionsToolsOverrides)]
#[case::case_7_stream_event_handler(Scenario::StreamEventHandler)]
#[case::case_8_assistants_beta_default(Scenario::AssistantsBetaDefault)]
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
