use std::str::from_utf8;

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_db::mcp_stored_contracts::decode::{
    McpStoredDecodeError, decode_mcp_stored_contract,
};
use betterlite_secrets::secret_codec::python_base64::decode_lenient;

use super::error::RotationCodecError;

pub fn decode_mcp_rotation_blob(bytes: &[u8]) -> Result<JsonValue, RotationCodecError> {
    let text = from_utf8(bytes).map_err(RotationCodecError::Utf8)?;
    match decode_mcp_stored_contract(bytes) {
        Ok(value) => Ok(value),
        Err(McpStoredDecodeError::Json(_)) => {
            let decoded = decode_lenient(text, true)?;
            String::from_utf8(decoded)
                .map(JsonValue::String)
                .map_err(|error| RotationCodecError::Utf8(error.utf8_error()))
        }
        Err(error) => Err(RotationCodecError::Blob(error)),
    }
}
