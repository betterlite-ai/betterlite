#![cfg(test)]

use rstest::rstest;

use crate::gemini_live_setup::catalog::{
    coerce_response_modalities, default_response_modality, is_audio_only, is_text_only,
};

use super::support::catalog;

#[test]
fn the_lookup_tries_the_exact_name_first() {
    let catalog = catalog();
    assert!(is_audio_only(&catalog, "audio-only-live"));
    assert!(is_audio_only(
        &catalog,
        "gemini-2.5-flash-native-audio-preview-09-2025"
    ));
    assert!(is_text_only(&catalog, "b/c"));
    assert!(is_text_only(
        &catalog,
        "vertex_ai/gemini-3.5-transcribe-live"
    ));
}

#[test]
fn the_lookup_strips_the_first_path_segment() {
    assert!(is_text_only(&catalog(), "prefix/b/c"));
}

#[test]
fn the_lookup_tries_the_gemini_prefix_after_the_stripped_name() {
    let catalog = catalog();
    assert!(is_text_only(&catalog, "gemini-3.5-transcribe-live"));
    assert!(is_text_only(&catalog, "x/gemini-3.5-transcribe-live"));
}

#[test]
fn the_stripped_name_is_checked_before_the_gemini_prefix() {
    let catalog = catalog();
    assert!(is_text_only(&catalog, "x/both-model"));
    assert!(!is_audio_only(&catalog, "x/both-model"));
}

#[test]
fn an_unknown_model_is_neither_audio_nor_text_only() {
    let catalog = catalog();
    assert!(!is_audio_only(&catalog, "missing-model"));
    assert!(!is_text_only(&catalog, "missing-model"));
    assert_eq!(
        default_response_modality(&catalog, "missing-model"),
        "AUDIO"
    );
}

#[rstest]
#[case::plain("gemini-2.5-flash", "AUDIO")]
#[case::native_audio("gemini-2.5-flash-native-audio-preview-09-2025", "AUDIO")]
#[case::audio_only("audio-only-live", "AUDIO")]
#[case::transcription("gemini-3.5-transcribe-live", "TEXT")]
#[case::vertex_transcription("vertex_ai/gemini-3.5-transcribe-live", "TEXT")]
fn the_default_modality_follows_the_catalog(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(default_response_modality(&catalog(), model), expected);
}

#[rstest]
#[case::native_audio_drops_text(
    "gemini-2.5-flash-native-audio-preview-09-2025",
    "text,audio",
    "AUDIO"
)]
#[case::native_audio_replaces_lone_text(
    "gemini-2.5-flash-native-audio-preview-09-2025",
    "text,TEXT",
    "AUDIO"
)]
#[case::native_audio_keeps_audio(
    "gemini-2.5-flash-native-audio-preview-09-2025",
    "audio,audio",
    "AUDIO,AUDIO"
)]
#[case::audio_only_live_drops_text("audio-only-live", "text", "AUDIO")]
#[case::transcription_drops_audio("gemini-3.5-transcribe-live", "audio,text", "TEXT")]
#[case::transcription_replaces_lone_audio("vertex_ai/gemini-3.5-transcribe-live", "AUDIO", "TEXT")]
#[case::transcription_keeps_text("gemini-3.5-transcribe-live", "text", "TEXT")]
#[case::plain_model_keeps_both("gemini-2.5-flash", "text,audio", "TEXT,AUDIO")]
#[case::plain_model_keeps_empty("gemini-2.5-flash", "", "")]
#[case::audio_only_model_keeps_empty("audio-only-live", "", "")]
fn coercing_response_modalities_swaps_what_the_model_cannot_produce(
    #[case] model: &str,
    #[case] modalities: &str,
    #[case] expected: &str,
) {
    let split = |text: &str| -> Vec<String> {
        text.split(',')
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(
        coerce_response_modalities(&catalog(), model, &split(modalities)),
        split(expected)
    );
}
