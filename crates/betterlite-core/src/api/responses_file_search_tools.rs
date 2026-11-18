pub mod error;
pub mod outputs;
pub mod preparation;
pub mod results;
pub mod synthesis;
pub mod tools;

mod tests;

use serde::Serialize;
use serde::de::DeserializeOwned;

use self::error::FileSearchToolsError;

// a struct serializes in declaration order and Json reads that text back in order, which to_value would sort
pub(super) fn wire<T, Value>(value: &Value) -> Result<T, FileSearchToolsError>
where
    T: DeserializeOwned,
    Value: Serialize + ?Sized,
{
    let text =
        serde_json::to_string(value).map_err(|source| FileSearchToolsError::Wire { source })?;
    serde_json::from_str(&text).map_err(|source| FileSearchToolsError::Wire { source })
}
