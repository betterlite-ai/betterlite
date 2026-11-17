use super::error::VeoOperationError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedOperation {
    RemixRequest,
    RemixResponse,
    ListRequest,
    ListResponse,
    DeleteRequest,
    DeleteResponse,
    CreateCharacterRequest,
    CreateCharacterResponse,
    GetCharacterRequest,
    GetCharacterResponse,
    ExtensionRequest,
    ExtensionResponse,
}

// the NotImplementedError message LiteLLM raises for each video method Vertex AI does not serve
impl From<UnsupportedOperation> for VeoOperationError {
    fn from(operation: UnsupportedOperation) -> Self {
        let message = match operation {
            UnsupportedOperation::RemixRequest => {
                "Video remix is not supported by Vertex AI Veo. Please use video_generation() to create new videos."
            }
            UnsupportedOperation::RemixResponse => "Video remix is not supported by Vertex AI Veo.",
            UnsupportedOperation::ListRequest => {
                "Video list is not supported by Vertex AI Veo. Use the operations endpoint directly if you need to list operations."
            }
            UnsupportedOperation::ListResponse => "Video list is not supported by Vertex AI Veo.",
            UnsupportedOperation::DeleteRequest => {
                "Video delete is not supported by Vertex AI Veo. Videos are automatically cleaned up by Google."
            }
            UnsupportedOperation::DeleteResponse => {
                "Video delete is not supported by Vertex AI Veo."
            }
            UnsupportedOperation::CreateCharacterRequest
            | UnsupportedOperation::CreateCharacterResponse => {
                "video create character is not supported for Vertex AI"
            }
            UnsupportedOperation::GetCharacterRequest
            | UnsupportedOperation::GetCharacterResponse => {
                "video get character is not supported for Vertex AI"
            }
            UnsupportedOperation::ExtensionRequest | UnsupportedOperation::ExtensionResponse => {
                "video extension is not supported for Vertex AI"
            }
        };
        Self::Unsupported { message }
    }
}
