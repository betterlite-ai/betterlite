#![cfg(test)]

use std::collections::HashSet;

use betterlite_core::routes::CallType;

use super::{
    COMPLETION_CALL_TYPES, EMBEDDINGS_CALL_TYPES, IMAGE_CALL_TYPES, SPEECH_CALL_TYPES,
    TRANSCRIPTION_CALL_TYPES, VIDEO_CALL_TYPES,
};

fn wire_names<const LENGTH: usize>(table: [CallType; LENGTH]) -> String {
    serde_json::to_string(table.as_slice()).unwrap()
}

#[test]
fn table_completion_lists_text_completion_then_atext_completion() {
    assert_eq!(
        COMPLETION_CALL_TYPES,
        [CallType::TextCompletion, CallType::AtextCompletion]
    );
    assert_eq!(
        wire_names(COMPLETION_CALL_TYPES),
        r#"["text_completion","atext_completion"]"#
    );
}

#[test]
fn table_embeddings_lists_embedding_then_aembedding() {
    assert_eq!(
        EMBEDDINGS_CALL_TYPES,
        [CallType::Embedding, CallType::Aembedding]
    );
    assert_eq!(
        wire_names(EMBEDDINGS_CALL_TYPES),
        r#"["embedding","aembedding"]"#
    );
}

#[test]
fn table_image_lists_image_generation_then_aimage_generation() {
    assert_eq!(
        IMAGE_CALL_TYPES,
        [CallType::ImageGeneration, CallType::AimageGeneration]
    );
    assert_eq!(
        wire_names(IMAGE_CALL_TYPES),
        r#"["image_generation","aimage_generation"]"#
    );
}

#[test]
fn table_speech_lists_speech_then_aspeech() {
    assert_eq!(SPEECH_CALL_TYPES, [CallType::Speech, CallType::Aspeech]);
    assert_eq!(wire_names(SPEECH_CALL_TYPES), r#"["speech","aspeech"]"#);
}

#[test]
fn table_transcription_lists_transcription_then_atranscription() {
    assert_eq!(
        TRANSCRIPTION_CALL_TYPES,
        [CallType::Transcription, CallType::Atranscription]
    );
    assert_eq!(
        wire_names(TRANSCRIPTION_CALL_TYPES),
        r#"["transcription","atranscription"]"#
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
    assert_eq!(
        wire_names(VIDEO_CALL_TYPES),
        r#"["video_generation","avideo_generation","create_video","acreate_video","video_remix","avideo_remix","video_edit","avideo_edit","video_extension","avideo_extension"]"#
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
