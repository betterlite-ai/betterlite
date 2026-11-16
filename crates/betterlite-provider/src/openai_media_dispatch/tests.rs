#![cfg(test)]

mod fixture;
mod image_edit;
mod image_generation;
mod image_variation;
mod speech;
mod transcription;
mod unsupported_operation;
mod video_create;
mod video_extended;
mod video_lifecycle;

use rstest::rstest;

#[derive(Clone, Copy)]
enum Family {
    ImageGeneration,
    ImageEdit,
    ImageVariation,
    Speech,
    Transcription,
    VideoCreate,
    VideoLifecycle,
    VideoExtendedOperations,
}

// One row per openai-media-dispatch oracle case file, each driving the production dispatcher
#[rstest]
#[case::image_generation(Family::ImageGeneration)]
#[case::image_edit(Family::ImageEdit)]
#[case::image_variation(Family::ImageVariation)]
#[case::speech(Family::Speech)]
#[case::transcription(Family::Transcription)]
#[case::video_create(Family::VideoCreate)]
#[case::video_lifecycle(Family::VideoLifecycle)]
#[case::video_extended_operations(Family::VideoExtendedOperations)]
#[tokio::test]
async fn openai_media_dispatch_cases(#[case] family: Family) {
    match family {
        Family::ImageGeneration => image_generation::check().await,
        Family::ImageEdit => image_edit::check().await,
        Family::ImageVariation => image_variation::check().await,
        Family::Speech => speech::check().await,
        Family::Transcription => transcription::check().await,
        Family::VideoCreate => video_create::check().await,
        Family::VideoLifecycle => video_lifecycle::check().await,
        Family::VideoExtendedOperations => video_extended::check().await,
    }
}
