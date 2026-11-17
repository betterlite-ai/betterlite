use super::error::StudioVeoOperationError;

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
    EditRequest,
    EditResponse,
    ExtensionRequest,
    ExtensionResponse,
}

// the NotImplementedError message LiteLLM raises for each video method Gemini does not serve
impl From<UnsupportedOperation> for StudioVeoOperationError {
    fn from(operation: UnsupportedOperation) -> Self {
        let message = match operation {
            UnsupportedOperation::RemixRequest => {
                "Video remix is not supported by Google Veo. Please use video_generation() to create new videos."
            }
            UnsupportedOperation::RemixResponse => "Video remix is not supported by Google Veo.",
            UnsupportedOperation::ListRequest => {
                "Video list is not supported by Google Veo. Use the operations endpoint directly if you need to list operations."
            }
            UnsupportedOperation::ListResponse => "Video list is not supported by Google Veo.",
            UnsupportedOperation::DeleteRequest => {
                "Video delete is not supported by Google Veo. Videos are automatically cleaned up by Google."
            }
            UnsupportedOperation::DeleteResponse => "Video delete is not supported by Google Veo.",
            UnsupportedOperation::CreateCharacterRequest
            | UnsupportedOperation::CreateCharacterResponse => {
                "video create character is not supported for Gemini"
            }
            UnsupportedOperation::GetCharacterRequest
            | UnsupportedOperation::GetCharacterResponse => {
                "video get character is not supported for Gemini"
            }
            UnsupportedOperation::EditRequest | UnsupportedOperation::EditResponse => {
                "video edit is not supported for Gemini"
            }
            UnsupportedOperation::ExtensionRequest | UnsupportedOperation::ExtensionResponse => {
                "video extension is not supported for Gemini"
            }
        };
        Self::Unsupported { message }
    }
}
