use serde_json::value::RawValue;

use super::JsonValue;
use super::decode::{McpStoredDecodeError, decode_mcp_stored_contract};

impl TryFrom<Box<RawValue>> for JsonValue {
    type Error = McpStoredDecodeError;

    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        decode_mcp_stored_contract(raw.get().as_bytes())
    }
}
