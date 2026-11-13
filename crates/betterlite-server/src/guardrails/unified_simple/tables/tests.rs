#![cfg(test)]

use std::collections::HashSet;

use betterlite_core::routes::CallType;

use super::{
    COMPLETION_CALL_TYPES, EMBEDDINGS_CALL_TYPES, IMAGE_CALL_TYPES, SPEECH_CALL_TYPES,
    TRANSCRIPTION_CALL_TYPES, VIDEO_CALL_TYPES,
};

#[test]
fn table_completion_lists_text_completion_then_atext_completion() {
    assert_eq!(
        COMPLETION_CALL_TYPES,
        [CallType::TextCompletion, CallType::AtextCompletion]
    );
}

#[test]
fn table_embeddings_lists_embedding_then_aembedding() {
    assert_eq!(
        EMBEDDINGS_CALL_TYPES,
        [CallType::Embedding, CallType::Aembedding]
    );
}

#[test]
fn table_image_lists_image_generation_then_aimage_generation() {
    assert_eq!(
        IMAGE_CALL_TYPES,
        [CallType::ImageGeneration, CallType::AimageGeneration]
    );
}

#[test]
fn table_speech_lists_speech_then_aspeech() {
    assert_eq!(SPEECH_CALL_TYPES, [CallType::Speech, CallType::Aspeech]);
}

#[test]
fn table_transcription_lists_transcription_then_atranscription() {
    assert_eq!(
        TRANSCRIPTION_CALL_TYPES,
        [CallType::Transcription, CallType::Atranscription]
    );
}

#[test]
fn table_video_lists_the_ten_pinned_call_types() {
    assert_eq!(
        VIDEO_CALL_TYPES,
        [
            CallType::VideoGeneration,
            CallType::AvideoGeneration,
            CallType::CreateVideo,
            CallType::AcreateVideo,
            CallType::VideoRemix,
            CallType::AvideoRemix,
            CallType::VideoEdit,
            CallType::AvideoEdit,
            CallType::VideoExtension,
            CallType::AvideoExtension,
        ]
    );
}

#[test]
fn table_call_types_appear_in_no_two_tables() {
    let all: HashSet<CallType> = COMPLETION_CALL_TYPES
        .into_iter()
        .chain(EMBEDDINGS_CALL_TYPES)
        .chain(IMAGE_CALL_TYPES)
        .chain(SPEECH_CALL_TYPES)
        .chain(TRANSCRIPTION_CALL_TYPES)
        .chain(VIDEO_CALL_TYPES)
        .collect();
    assert_eq!(all.len(), 20);
}
