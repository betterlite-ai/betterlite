use rust_decimal::Decimal;

use super::support::{AUDIO_CONTENT, CONTENT, PROMPT, session};
use crate::realtime_session::records::{
    AudioSettings, ExactDecimal, MaxResponseTokens, Setting, UsageEvent,
};

const USAGE_ZERO: &str = concat!(
    r#"{"totalInputTokens":0,"totalOutputTokens":0,"totalTokens":0,"details":"#,
    r#"{"total":{"input":{"speechTokens":0,"textTokens":0},"output":{"speechTokens":0,"textTokens":0}}}}"#,
);

pub(super) fn default_session_configuration() {
    default_session_state();
    default_inference_configuration();
    default_prompt_configuration();
}

fn default_session_state() {
    let state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    assert_eq!(state.names.prompt, PROMPT);
    assert_eq!(state.names.content, CONTENT);
    assert_eq!(state.names.audio_content, AUDIO_CONTENT);
    assert_ne!(state.names.prompt, state.names.content);
    assert_ne!(state.names.content, state.names.audio_content);
    assert_ne!(state.names.prompt, state.names.audio_content);
    assert!(!state.prompt_started);
    assert!(!state.client_audio_streamed);
    assert!(!state.transcript.active);
    assert_eq!(state.transcript.generation_stage, None);
    assert_eq!(state.transcript.item_id, None);
    assert_eq!(state.transcript.buffer, "");
    assert_usage_zero(&state.cumulative_usage);
    assert_usage_zero(&state.reported_usage);
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
    let audio = &prompt_start.audio_output_configuration;
    assert_eq!(audio.media_type, "audio/lpcm");
    assert_eq!(audio.sample_rate_hertz, Setting(Some(24000)));
    assert_eq!(audio.sample_size_bits, 16);
    assert_eq!(audio.channel_count, 1);
    assert_eq!(audio.voice_id, Setting(Some("matthew".to_owned())));
    assert_eq!(audio.encoding, "base64");
    assert_eq!(audio.audio_type, "SPEECH");
    assert!(prompt_start.tool_use_output_configuration.is_none());
    assert!(prompt_start.tool_configuration.is_none());
    let wire = serde_json::to_string(&configuration).expect("the marker serializes");
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
