use super::support::{AUDIO_CONTENT, CONTENT, PROMPT, apply, output_audio, session};
use crate::realtime_session::RealtimeSession;
use crate::realtime_session::records::{Setting, UpdateFrame};

pub(super) fn pcm16_input_output_rates() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let mut frames = apply(
        &mut state,
        r#"{"session":{"output_audio_format":"g711_ulaw","input_audio_format":"g711_ulaw"}}"#,
        &[],
    );
    assert_eq!(output_audio(&frames).sample_rate_hertz, Setting(Some(8000)));
    assert_eq!(state.input_audio.sample_rate_hertz, Setting(Some(8000)));
    frames = apply(
        &mut state,
        r#"{"session":{"output_audio_format":"pcm16","input_audio_format":"pcm16"}}"#,
        &[],
    );
    assert_pcm_defaults(&state, &frames);
    frames = apply(
        &mut state,
        r#"{"session":{"output_audio_format":"opus","input_audio_format":"opus"}}"#,
        &[],
    );
    assert_pcm_defaults(&state, &frames);
    frames = apply(
        &mut state,
        r#"{"session":{"output_audio_format":null,"input_audio_format":null}}"#,
        &[],
    );
    assert_pcm_defaults(&state, &frames);
}

pub(super) fn g711_rates() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let mut frames = apply(
        &mut state,
        r#"{"session":{"output_audio_format":"g711_ulaw","input_audio_format":"g711_ulaw"}}"#,
        &[],
    );
    assert_eq!(output_audio(&frames).sample_rate_hertz, Setting(Some(8000)));
    assert_eq!(state.input_audio.sample_rate_hertz, Setting(Some(8000)));
    assert_eq!(output_audio(&frames).media_type, "audio/lpcm");
    assert_eq!(output_audio(&frames).sample_size_bits, 16);
    assert_eq!(output_audio(&frames).encoding, "base64");
    assert_eq!(output_audio(&frames).audio_type, "SPEECH");
    frames = apply(
        &mut state,
        r#"{"session":{"output_audio_format":"g711_alaw","input_audio_format":"g711_alaw"}}"#,
        &[],
    );
    assert_eq!(output_audio(&frames).sample_rate_hertz, Setting(Some(8000)));
    assert_eq!(state.input_audio.sample_rate_hertz, Setting(Some(8000)));
    assert_eq!(output_audio(&frames).media_type, "audio/lpcm");
    assert_eq!(output_audio(&frames).sample_size_bits, 16);
    assert_eq!(output_audio(&frames).encoding, "base64");
}

pub(super) fn explicit_sample_rate() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let mut frames = apply(
        &mut state,
        r#"{"session":{"output_sample_rate_hertz":48000,"input_sample_rate_hertz":22050}}"#,
        &[],
    );
    assert_eq!(
        output_audio(&frames).sample_rate_hertz,
        Setting(Some(48000))
    );
    assert_eq!(state.input_audio.sample_rate_hertz, Setting(Some(22050)));
    frames = apply(
        &mut state,
        r#"{"session":{"output_sample_rate_hertz":0,"input_sample_rate_hertz":0}}"#,
        &[],
    );
    assert_eq!(output_audio(&frames).sample_rate_hertz, Setting(Some(0)));
    assert_eq!(state.input_audio.sample_rate_hertz, Setting(Some(0)));
    frames = apply(
        &mut state,
        r#"{"session":{"output_sample_rate_hertz":null,"input_sample_rate_hertz":null}}"#,
        &[],
    );
    assert_eq!(output_audio(&frames).sample_rate_hertz, Setting(None));
    assert_eq!(state.input_audio.sample_rate_hertz, Setting(None));
    explicit_rates_override_formats();
}

pub(super) fn explicit_rates_override_formats() {
    let mut state = session(&[PROMPT, CONTENT, AUDIO_CONTENT]);
    let mut frames = apply(
        &mut state,
        r#"{"session":{"input_audio_format":"g711_ulaw","input_sample_rate_hertz":22050,"output_audio_format":"g711_ulaw","output_sample_rate_hertz":12345}}"#,
        &[],
    );
    assert_eq!(
        output_audio(&frames).sample_rate_hertz,
        Setting(Some(12345))
    );
    assert_eq!(state.input_audio.sample_rate_hertz, Setting(Some(22050)));
    assert_eq!(output_audio(&frames).media_type, "audio/lpcm");
    frames = apply(
        &mut state,
        r#"{"session":{"output_sample_rate_hertz":24000}}"#,
        &[],
    );
    assert_eq!(
        output_audio(&frames).sample_rate_hertz,
        Setting(Some(24000))
    );
    assert_eq!(state.input_audio.sample_rate_hertz, Setting(Some(22050)));
}

fn assert_pcm_defaults(state: &RealtimeSession, frames: &[UpdateFrame]) {
    let audio = output_audio(frames);
    assert_eq!(audio.sample_rate_hertz, Setting(Some(24000)));
    assert_eq!(audio.media_type, "audio/lpcm");
    assert_eq!(audio.sample_size_bits, 16);
    assert_eq!(audio.channel_count, 1);
    assert_eq!(audio.encoding, "base64");
    assert_eq!(audio.audio_type, "SPEECH");
    assert_eq!(state.input_audio.sample_rate_hertz, Setting(Some(16000)));
}
