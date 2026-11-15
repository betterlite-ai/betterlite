#![cfg(test)]

mod audio_cases;
mod conversation_cases;
mod corpus;
mod support;
mod trigger_cases;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputCase {
    AudioAppendBeforeAfterSession,
    AudioCommit,
    ConversationTextItem,
    ResponseCreateReadyAudio,
    ResponseCancel,
    CloseSessionOrder,
    UnknownClientEvent,
    AudioSampleRateTransition,
    ConversationToolResult,
    ReadyTriggerFixture,
}

#[rstest::rstest]
#[case::audio_append_before_after_session(InputCase::AudioAppendBeforeAfterSession)]
#[case::audio_commit(InputCase::AudioCommit)]
#[case::conversation_text_item(InputCase::ConversationTextItem)]
#[case::response_create_ready_audio(InputCase::ResponseCreateReadyAudio)]
#[case::response_cancel(InputCase::ResponseCancel)]
#[case::close_session_order(InputCase::CloseSessionOrder)]
#[case::unknown_client_event(InputCase::UnknownClientEvent)]
#[case::audio_sample_rate_transition(InputCase::AudioSampleRateTransition)]
#[case::conversation_tool_result(InputCase::ConversationToolResult)]
#[case::ready_trigger_fixture(InputCase::ReadyTriggerFixture)]
fn realtime_input_cases(#[case] case: InputCase) {
    match case {
        InputCase::AudioAppendBeforeAfterSession => {
            audio_cases::audio_append_before_after_session()
        }
        InputCase::AudioCommit => audio_cases::audio_commit(),
        InputCase::ConversationTextItem => conversation_cases::conversation_text_item(),
        InputCase::ResponseCreateReadyAudio => trigger_cases::response_create_ready_audio(),
        InputCase::ResponseCancel => trigger_cases::response_cancel(),
        InputCase::CloseSessionOrder => audio_cases::close_session_order(),
        InputCase::UnknownClientEvent => trigger_cases::unknown_client_event(),
        InputCase::AudioSampleRateTransition => audio_cases::audio_sample_rate_transition(),
        InputCase::ConversationToolResult => conversation_cases::conversation_tool_result(),
        InputCase::ReadyTriggerFixture => trigger_cases::ready_trigger_fixture(),
    }
}
