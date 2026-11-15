#![cfg(test)]

mod assistant_cases;
mod corpus;
mod delta_cases;
mod stage_cases;
mod support;
mod user_cases;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputCase {
    AssistantContentStart,
    GenerationStage,
    UserSpeechEvent,
    UserTranscriptFinal,
    SpeculativeText,
    FinalText,
    AudioOutputDelta,
    ItemIdConsistency,
    AssistantEventOrder,
}

#[rstest::rstest]
#[case::assistant_content_start(OutputCase::AssistantContentStart)]
#[case::generation_stage(OutputCase::GenerationStage)]
#[case::user_speech_event(OutputCase::UserSpeechEvent)]
#[case::user_transcript_final(OutputCase::UserTranscriptFinal)]
#[case::speculative_text(OutputCase::SpeculativeText)]
#[case::final_text(OutputCase::FinalText)]
#[case::audio_output_delta(OutputCase::AudioOutputDelta)]
#[case::item_id_consistency(OutputCase::ItemIdConsistency)]
#[case::assistant_event_order(OutputCase::AssistantEventOrder)]
fn realtime_output_cases(#[case] case: OutputCase) {
    match case {
        OutputCase::AssistantContentStart => assistant_cases::assistant_content_start(),
        OutputCase::GenerationStage => stage_cases::generation_stage(),
        OutputCase::UserSpeechEvent => user_cases::user_speech_event(),
        OutputCase::UserTranscriptFinal => user_cases::user_transcript_final(),
        OutputCase::SpeculativeText => user_cases::speculative_text(),
        OutputCase::FinalText => delta_cases::final_text(),
        OutputCase::AudioOutputDelta => delta_cases::audio_output_delta(),
        OutputCase::ItemIdConsistency => user_cases::item_id_consistency(),
        OutputCase::AssistantEventOrder => assistant_cases::assistant_event_order(),
    }
}
