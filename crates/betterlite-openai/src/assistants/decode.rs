use crate::assistants::error::AssistantsError;
use crate::assistants::records::AssistantObject;

pub fn decode_assistant_object(bytes: &[u8]) -> Result<AssistantObject, AssistantsError> {
    match serde_json::from_slice(bytes) {
        Ok(assistant) => Ok(assistant),
        Err(source) => Err(AssistantsError::from(source)),
    }
}
