use rstest::rstest;

use crate::vertex_veo_operation::error::VeoOperationError;
use crate::vertex_veo_operation::unsupported::UnsupportedOperation;

#[rstest]
#[case::remix_request(
    UnsupportedOperation::RemixRequest,
    "Video remix is not supported by Vertex AI Veo. Please use video_generation() to create new videos."
)]
#[case::remix_response(
    UnsupportedOperation::RemixResponse,
    "Video remix is not supported by Vertex AI Veo."
)]
#[case::list_request(
    UnsupportedOperation::ListRequest,
    "Video list is not supported by Vertex AI Veo. Use the operations endpoint directly if you need to list operations."
)]
#[case::list_response(
    UnsupportedOperation::ListResponse,
    "Video list is not supported by Vertex AI Veo."
)]
#[case::delete_request(
    UnsupportedOperation::DeleteRequest,
    "Video delete is not supported by Vertex AI Veo. Videos are automatically cleaned up by Google."
)]
#[case::delete_response(
    UnsupportedOperation::DeleteResponse,
    "Video delete is not supported by Vertex AI Veo."
)]
#[case::create_character_request(
    UnsupportedOperation::CreateCharacterRequest,
    "video create character is not supported for Vertex AI"
)]
#[case::create_character_response(
    UnsupportedOperation::CreateCharacterResponse,
    "video create character is not supported for Vertex AI"
)]
#[case::get_character_request(
    UnsupportedOperation::GetCharacterRequest,
    "video get character is not supported for Vertex AI"
)]
#[case::get_character_response(
    UnsupportedOperation::GetCharacterResponse,
    "video get character is not supported for Vertex AI"
)]
#[case::extension_request(
    UnsupportedOperation::ExtensionRequest,
    "video extension is not supported for Vertex AI"
)]
#[case::extension_response(
    UnsupportedOperation::ExtensionResponse,
    "video extension is not supported for Vertex AI"
)]
fn each_unsupported_video_method_keeps_its_message(
    #[case] operation: UnsupportedOperation,
    #[case] message: &str,
) {
    let error = VeoOperationError::from(operation);
    assert_eq!(error.to_string(), message);
}
