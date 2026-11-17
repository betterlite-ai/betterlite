use super::fixture::{Fixture, Recorder, media_error};
use crate::openai_dispatch::OpenAiRequest;
use crate::openai_media_dispatch::call::{
    OpenAiMediaOperation, VideoCall, VideoOperation, VideoStatusCall,
};
use crate::openai_media_dispatch::error::OpenAiMediaError;
use crate::openai_media_dispatch::execute::dispatch;
use crate::operation::OperationRequest;

#[tokio::test]
async fn openai_media_dispatch_unsupported_operation() {
    let fixture = Fixture::new(200, b"");
    let mut logger = Recorder;
    let operation = OpenAiMediaOperation::Video(Box::new(VideoOperation {
        api_base: None,
        provider: None,
        call: VideoCall::Status(Box::new(VideoStatusCall {
            video_id: String::from("video_1"),
            reply: Vec::new(),
        })),
    }));
    let error = media_error(
        dispatch(
            OperationRequest::OpenAi(OpenAiRequest),
            operation,
            &fixture,
            &mut logger,
        )
        .await,
    );
    assert!(matches!(error, OpenAiMediaError::UnsupportedOperation));
    assert_eq!(
        error.to_string(),
        "the openai media composition runs only image, speech, transcription and video operations"
    );
    assert_eq!(fixture.request_count(), 0);
}
