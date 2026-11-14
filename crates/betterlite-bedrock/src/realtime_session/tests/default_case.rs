use rust_decimal::Decimal;

use super::support::{AUDIO_CONTENT, CONTENT, PROMPT, session};
use crate::realtime_session::records::{ExactDecimal, MaxResponseTokens, Setting, UsageEvent};

pub(super) fn default_session_configuration() {
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
    assert_eq!(state.cumulative_usage, UsageEvent::default());
    assert_eq!(state.reported_usage, UsageEvent::default());
    assert_eq!(state.cumulative_usage.total_input_tokens, 0);
    assert_eq!(state.cumulative_usage.total_output_tokens, 0);
    assert_eq!(state.cumulative_usage.total_tokens, 0);
    assert_eq!(state.cumulative_usage.details.total.input.speech_tokens, 0);
    assert_eq!(state.cumulative_usage.details.total.input.text_tokens, 0);
    assert_eq!(state.cumulative_usage.details.total.output.speech_tokens, 0);
    assert_eq!(state.cumulative_usage.details.total.output.text_tokens, 0);
    assert_eq!(state.reported_usage.details.total.input.speech_tokens, 0);
    assert_eq!(state.reported_usage.details.total.input.text_tokens, 0);
    assert_eq!(state.reported_usage.details.total.output.speech_tokens, 0);
    assert_eq!(state.reported_usage.details.total.output.text_tokens, 0);
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
    assert_eq!(state.output_audio.sample_rate_hertz, Setting(Some(24000)));
    assert_eq!(state.output_audio.sample_size_bits, 16);
    assert_eq!(state.output_audio.channel_count, 1);
    assert_eq!(state.output_audio.encoding, "base64");
    assert_eq!(state.output_audio.audio_type, "SPEECH");
    assert_eq!(state.output_audio.media_type, "audio/lpcm");
    assert_eq!(state.input_audio.sample_rate_hertz, Setting(Some(16000)));
    assert_eq!(state.input_audio.sample_size_bits, 16);
    assert_eq!(state.input_audio.channel_count, 1);
    assert_eq!(state.input_audio.encoding, "base64");
    assert_eq!(state.input_audio.audio_type, "SPEECH");
    assert_eq!(state.input_audio.media_type, "audio/lpcm");
    assert_eq!(state.text_media_type, "text/plain");

    let configuration = state.configuration_request(None);
    let inference = &configuration
        .session_start
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
    let prompt_start = &configuration.prompt_start.prompt_start;
    assert_eq!(prompt_start.prompt_name, PROMPT);
    assert_eq!(
        prompt_start.text_output_configuration.media_type,
        "text/plain"
    );
    assert_eq!(
        prompt_start.audio_output_configuration.media_type,
        "audio/lpcm"
    );
    assert_eq!(
        prompt_start.audio_output_configuration.sample_rate_hertz,
        Setting(Some(24000))
    );
    assert_eq!(prompt_start.audio_output_configuration.sample_size_bits, 16);
    assert_eq!(prompt_start.audio_output_configuration.channel_count, 1);
    assert_eq!(
        prompt_start.audio_output_configuration.voice_id,
        Setting(Some("matthew".to_owned()))
    );
    assert_eq!(prompt_start.audio_output_configuration.encoding, "base64");
    assert_eq!(prompt_start.audio_output_configuration.audio_type, "SPEECH");
    assert!(prompt_start.tool_use_output_configuration.is_none());
    assert!(prompt_start.tool_configuration.is_none());
    assert!(!state.prompt_started);

    let wire = serde_json::to_string(&configuration).expect("the marker serializes");
    assert!(wire.contains(r#""session_start""#));
    assert!(wire.contains(r#""prompt_start""#));
    assert!(wire.contains(r#""sessionStart""#));
    assert!(wire.contains(r#""promptStart""#));
    assert!(wire.contains(r#""maxTokens":1024"#));
    assert!(wire.contains(r#""topP":0.9"#));
    assert!(wire.contains(r#""temperature":0.7"#));
    assert!(wire.contains(r#""mediaType":"text/plain""#));
    assert!(wire.contains(r#""mediaType":"audio/lpcm""#));
    assert!(wire.contains(r#""sampleRateHertz":24000"#));
    assert!(wire.contains(r#""sampleSizeBits":16"#));
    assert!(wire.contains(r#""channelCount":1"#));
    assert!(wire.contains(r#""voiceId":"matthew""#));
    assert!(wire.contains(r#""encoding":"base64""#));
    assert!(wire.contains(r#""audioType":"SPEECH""#));
}
