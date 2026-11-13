use rstest::rstest;

use super::create_request;
use crate::batches::records::BatchEndpoint;
use crate::xai_batches::transformation::create_endpoint;

#[rstest]
#[case::default(None, "/v1/chat/completions")]
#[case::chat(Some(BatchEndpoint::ChatCompletions), "/v1/chat/completions")]
#[case::embeddings(Some(BatchEndpoint::Embeddings), "/v1/embeddings")]
#[case::completions(Some(BatchEndpoint::Completions), "/v1/completions")]
#[case::responses(Some(BatchEndpoint::Responses), "/v1/responses")]
#[case::ocr(Some(BatchEndpoint::Ocr), "/v1/ocr")]
#[case::image_generations(Some(BatchEndpoint::ImageGenerations), "/v1/images/generations")]
#[case::image_edits(Some(BatchEndpoint::ImageEdits), "/v1/images/edits")]
#[case::video_generations(Some(BatchEndpoint::VideoGenerations), "/v1/videos/generations")]
#[case::videos(Some(BatchEndpoint::Videos), "/v1/videos")]
#[case::video_edits(Some(BatchEndpoint::VideoEdits), "/v1/videos/edits")]
#[case::video_extensions(Some(BatchEndpoint::VideoExtensions), "/v1/videos/extensions")]
fn create_endpoint_defaults_and_keeps_the_caller_value(
    #[case] endpoint: Option<BatchEndpoint>,
    #[case] expected: &str,
) {
    let mut request = create_request(Some("file-1"), None);
    request.endpoint = endpoint;
    assert_eq!(create_endpoint(&request), expected);
}
