#![cfg(test)]

mod content_cases;
mod corpus;
mod support;
mod tool_cases;
mod usage_cases;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionCase {
    ContentEndTextAudio,
    PromptEndResponseDone,
    IncrementalUsageDelta,
    LeftoverUsageDone,
    ToolUseEvent,
    MultipleContentBlocks,
    OutputItemCompletion,
}

#[rstest::rstest]
#[case::content_end_text_audio(CompletionCase::ContentEndTextAudio)]
#[case::prompt_end_response_done(CompletionCase::PromptEndResponseDone)]
#[case::incremental_usage_delta(CompletionCase::IncrementalUsageDelta)]
#[case::leftover_usage_done(CompletionCase::LeftoverUsageDone)]
#[case::tool_use_event(CompletionCase::ToolUseEvent)]
#[case::multiple_content_blocks(CompletionCase::MultipleContentBlocks)]
#[case::output_item_completion(CompletionCase::OutputItemCompletion)]
fn realtime_completion_cases(#[case] case: CompletionCase) {
    match case {
        CompletionCase::ContentEndTextAudio => content_cases::content_end_text_audio(),
        CompletionCase::PromptEndResponseDone => usage_cases::prompt_end_response_done(),
        CompletionCase::IncrementalUsageDelta => usage_cases::incremental_usage_delta(),
        CompletionCase::LeftoverUsageDone => usage_cases::leftover_usage_done(),
        CompletionCase::ToolUseEvent => tool_cases::tool_use_event(),
        CompletionCase::MultipleContentBlocks => content_cases::multiple_content_blocks(),
        CompletionCase::OutputItemCompletion => content_cases::output_item_completion(),
    }
}
