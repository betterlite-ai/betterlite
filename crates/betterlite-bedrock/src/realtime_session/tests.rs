#![cfg(test)]

mod audio_cases;
mod default_case;
mod event_cases;
mod support;
mod tool_cases;
mod update_cases;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionCase {
    DefaultSessionConfiguration,
    SessionUpdateOptions,
    Pcm16InputOutputRates,
    G711Rates,
    ExplicitSampleRate,
    ToolConfiguration,
    SystemInstructions,
    CreatedUpdatedEvents,
}

#[rstest::rstest]
#[case::default_session_configuration(SessionCase::DefaultSessionConfiguration)]
#[case::session_update_options(SessionCase::SessionUpdateOptions)]
#[case::pcm16_input_output_rates(SessionCase::Pcm16InputOutputRates)]
#[case::g711_rates(SessionCase::G711Rates)]
#[case::explicit_sample_rate(SessionCase::ExplicitSampleRate)]
#[case::tool_configuration(SessionCase::ToolConfiguration)]
#[case::system_instructions(SessionCase::SystemInstructions)]
#[case::created_updated_events(SessionCase::CreatedUpdatedEvents)]
fn realtime_session_cases(#[case] case: SessionCase) {
    match case {
        SessionCase::DefaultSessionConfiguration => {
            default_case::default_session_configuration();
        }
        SessionCase::SessionUpdateOptions => update_cases::session_update_options(),
        SessionCase::Pcm16InputOutputRates => audio_cases::pcm16_input_output_rates(),
        SessionCase::G711Rates => audio_cases::g711_rates(),
        SessionCase::ExplicitSampleRate => audio_cases::explicit_sample_rate(),
        SessionCase::ToolConfiguration => tool_cases::tool_configuration(),
        SessionCase::SystemInstructions => update_cases::system_instructions(),
        SessionCase::CreatedUpdatedEvents => event_cases::created_updated_events(),
    }
}
