use serde::de::DeserializeOwned;
use serde_json::error::Category;

use crate::assistants::error::AssistantsError;

pub fn decode_assistant_object<T>(bytes: &[u8]) -> Result<T, AssistantsError>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|source| match source.classify() {
        Category::Eof => AssistantsError::Truncated { source },
        Category::Data => AssistantsError::Shape { source },
        Category::Syntax | Category::Io => AssistantsError::Malformed { source },
    })
}
