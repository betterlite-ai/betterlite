use betterlite_db::mcp_stored_contracts::{JsonValue, MCPCredentials, pinned};
use serde::Deserialize;
use serde_json::Value;

pub(crate) struct Op;

fn helper_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    let error = Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    });
    if class.is_empty() {
        return error;
    }
    error
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    CredentialContract {
        value: Value,
    },
    PinnedToolContract {
        value: Value,
    },
    ProtocolMetadata {
        value: serde_json::Map<String, Value>,
    },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let value = match input {
            Input::CredentialContract { value } => serde_json::to_value(
                serde_json::from_value::<MCPCredentials>(value.clone())
                    .map_err(|error| helper_error("ValidationError", error.to_string()))?,
            ),
            Input::PinnedToolContract { value } => {
                let decoded = serde_json::from_value::<JsonValue>(value.clone())
                    .map_err(|error| helper_error("ValidationError", error.to_string()))?;
                serde_json::to_value(
                    pinned::parse_pinned_tools(decoded)
                        .map_err(|error| helper_error("ValidationError", error.to_string()))?,
                )
            }
            Input::ProtocolMetadata { value } => {
                let encoded = serde_json::to_string(value)
                    .map_err(|error| helper_error("ValidationError", error.to_string()))?;
                let metadata =
                    serde_json::from_str::<std::collections::BTreeMap<String, JsonValue>>(&encoded)
                        .map_err(|error| helper_error("ValidationError", error.to_string()))?;
                serde_json::to_value(
                    pinned::validate_mcp_protocol_metadata(metadata)
                        .map_err(|error| helper_error("ValidationError", error.to_string()))?,
                )
            }
        };
        value.map_err(|error| helper_error("SerializationError", error.to_string()))
    }
}
