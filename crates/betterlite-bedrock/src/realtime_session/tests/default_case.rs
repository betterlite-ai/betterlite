use rust_decimal::Decimal;

use super::support::{AUDIO_CONTENT, CONTENT, PROMPT, session};
use crate::realtime_session::records::{
    AudioOutputConfiguration, AudioSettings, BedrockContentEnd, ExactDecimal, MaxResponseTokens,
    SessionConfiguration, Setting, UsageDetails, UsageEvent, UsageTokens,
};

const USAGE_ZERO: &str = concat!(
    r#"{"totalInputTokens":0,"totalOutputTokens":0,"totalTokens":0,"details":"#,
    r#"{"total":{"input":{"speechTokens":0,"textTokens":0},"output":{"speechTokens":0,"textTokens":0}}}}"#,
);

pub(super) fn default_session_configuration() {
    default_session_state();
    default_inference_configuration();
    default_prompt_configuration();
    partial_usage_event_zero_fills();
    content_end_defaults();
}

fn partial_usage_event_zero_fills() {
    let usage: UsageEvent =
        serde_json::from_str(r#"{"totalInputTokens":5}"#).expect("a partial usage event");
    assert_eq!(usage.total_input_tokens, 5);
    assert_eq!(usage.total_output_tokens, 0);
    assert_eq!(usage.total_tokens, 0);
    assert_eq!(usage.details, UsageDetails::default());
    let nested: UsageEvent =
        serde_json::from_str(r#"{"details":{"total":{"input":{"speechTokens":3}}}}"#)
            .expect("a nested partial usage event");
    assert_eq!(nested.details.total.input.speech_tokens, 3);
    assert_eq!(nested.details.total.output, UsageTokens::default());
}

fn content_end_defaults() {
    let empty: BedrockContentEnd = serde_json::from_str("{}").expect("an empty content end");
    assert_eq!(empty, BedrockContentEnd::default());
    let stopped: BedrockContentEnd =
        serde_json::from_str(r#"{"stopReason":"END_TURN"}"#).expect("a stopped content end");
    assert_eq!(stopped.stop_reason, Some("END_TURN".to_owned()));
}

fn default_session_state() {
    default_session_names();
    default_session_activity();
    default_session_settings();
}

fn default_session_names() {
    let state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    assert_eq!(state.names.prompt, PROMPT);
    assert_eq!(state.names.content, CONTENT);
    assert_eq!(state.names.audio_content, AUDIO_CONTENT);
    assert_ne!(state.names.prompt, state.names.content);
    assert_ne!(state.names.content, state.names.audio_content);
    assert_ne!(state.names.prompt, state.names.audio_content);
}

fn default_session_activity() {
    let state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    assert!(!state.prompt_started);
    assert!(!state.client_audio_streamed);
    assert!(!state.transcript.active);
    assert_eq!(state.transcript.generation_stage, None);
    assert_eq!(state.transcript.item_id, None);
    assert_eq!(state.transcript.buffer, "");
    assert_usage_zero(&state.cumulative_usage);
    assert_usage_zero(&state.reported_usage);
}

fn default_session_settings() {
    let state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    assert_eq!(
        state.max_tokens,
        Setting(Some(MaxResponseTokens::Count(1024)))
    );
    assert_eq!(state.top_p, ExactDecimal(Decimal::new(9, 1)));
    assert_eq!(
        state.temperature,
        Setting(Some(ExactDecimal(Decimal::new(7, 1))))
    );
    assert_eq!(state.voice_id, Setting(Some("matthew".to_owned())));
    assert_eq!(state.output_audio, expected_audio(24000));
    assert_eq!(state.input_audio, expected_audio(16000));
    assert_eq!(state.text_media_type, "text/plain");
}

fn default_inference_configuration() {
    let state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let configuration = state.configuration_request(None);
    let inference = &configuration
        .session_start
        .event
        .session_start
        .inference_configuration;
    assert_eq!(
        inference.max_tokens,
        Setting(Some(MaxResponseTokens::Count(1024)))
    );
    assert_eq!(inference.top_p, ExactDecimal(Decimal::new(9, 1)));
    assert_eq!(
        inference.temperature,
        Setting(Some(ExactDecimal(Decimal::new(7, 1))))
    );
    assert!(!state.prompt_started);
    let wire = serde_json::to_string(&configuration).expect("the marker serializes");
    assert!(wire.contains(r#""session_start":{"event":{"sessionStart""#));
    assert!(wire.contains(r#""maxTokens":1024"#));
    assert!(wire.contains(r#""topP":0.9"#));
    assert!(wire.contains(r#""temperature":0.7"#));
}

fn default_prompt_configuration() {
    let state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let configuration = state.configuration_request(None);
    let prompt_start = &configuration.prompt_start.event.prompt_start;
    assert_eq!(prompt_start.prompt_name, PROMPT);
    assert_eq!(
        prompt_start.text_output_configuration.media_type,
        "text/plain"
    );
    assert_audio_output(&prompt_start.audio_output_configuration);
    assert!(prompt_start.tool_use_output_configuration.is_none());
    assert!(prompt_start.tool_configuration.is_none());
    assert_prompt_wire(&configuration);
}

fn assert_audio_output(audio: &AudioOutputConfiguration) {
    assert_eq!(audio.media_type, "audio/lpcm");
    assert_eq!(audio.sample_rate_hertz, Setting(Some(24000)));
    assert_eq!(audio.sample_size_bits, 16);
    assert_eq!(audio.channel_count, 1);
    assert_eq!(audio.voice_id, Setting(Some("matthew".to_owned())));
    assert_eq!(audio.encoding, "base64");
    assert_eq!(audio.audio_type, "SPEECH");
}

fn assert_prompt_wire(configuration: &SessionConfiguration) {
    let wire = serde_json::to_string(configuration).expect("the marker serializes");
    assert!(wire.contains(r#""prompt_start":{"event":{"promptStart""#));
    assert!(wire.contains(r#""mediaType":"text/plain""#));
    assert!(wire.contains(r#""mediaType":"audio/lpcm""#));
    assert!(wire.contains(r#""sampleRateHertz":24000"#));
    assert!(wire.contains(r#""sampleSizeBits":16"#));
    assert!(wire.contains(r#""channelCount":1"#));
    assert!(wire.contains(r#""voiceId":"matthew""#));
    assert!(wire.contains(r#""encoding":"base64""#));
    assert!(wire.contains(r#""audioType":"SPEECH""#));
}

fn assert_usage_zero(event: &UsageEvent) {
    assert_eq!(event, &UsageEvent::default());
    assert_eq!(
        serde_json::to_string(event).expect("usage serializes"),
        USAGE_ZERO
    );
}

fn expected_audio(sample_rate_hertz: i64) -> AudioSettings {
    AudioSettings {
        sample_rate_hertz: Setting(Some(sample_rate_hertz)),
        sample_size_bits: 16,
        channel_count: 1,
        encoding: "base64".to_owned(),
        audio_type: "SPEECH".to_owned(),
        media_type: "audio/lpcm".to_owned(),
    }
}
