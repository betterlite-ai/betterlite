use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::{
    JsonValue, MCPAuthType, MCPCredentials, MCPTokenEndpointAuthMethod, MCPTransport, decode,
    pinned,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    steps: BTreeMap<String, Call>,
}

#[derive(Deserialize)]
#[serde(tag = "function", content = "value", rename_all = "snake_case")]
enum Call {
    CredentialContract(Value),
    Transport(Value),
    Auth(Value),
    TokenEndpointMethod(Value),
    PinnedToolContract(Value),
    ProtocolMetadata(Value),
}

#[derive(Serialize)]
struct Outcome {
    value: Option<Value>,
    error: Option<&'static str>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let results: BTreeMap<_, _> = input
            .steps
            .iter()
            .map(|(name, call)| {
                let result = match call {
                    Call::CredentialContract(value) => validate::<MCPCredentials>(value),
                    Call::Transport(value) => validate::<MCPTransport>(value),
                    Call::Auth(value) => validate::<MCPAuthType>(value),
                    Call::TokenEndpointMethod(value) => {
                        validate::<MCPTokenEndpointAuthMethod>(value)
                    }
                    Call::PinnedToolContract(value) => parse_tools(value),
                    Call::ProtocolMetadata(value) => parse_metadata(value),
                };
                let outcome = match result {
                    Ok(value) => Outcome {
                        value: Some(value),
                        error: None,
                    },
                    Err(error) => Outcome {
                        value: None,
                        error: Some(error),
                    },
                };
                (name, outcome)
            })
            .collect();
        serde_json::to_value(results).map_err(|source| {
            Box::new(crate::helpers::HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
    }
}

fn validate<Record>(value: &Value) -> Result<Value, &'static str>
where
    Record: DeserializeOwned + Serialize,
{
    let record: Record = serde_json::from_value(value.clone()).map_err(|_| "ValidationError")?;
    serde_json::to_value(record).map_err(|_| "SerializationError")
}

fn parse_tools(value: &Value) -> Result<Value, &'static str> {
    let bytes = serde_json::to_vec(value).map_err(|_| "SerializationError")?;
    let decoded = decode::decode_mcp_stored_contract(&bytes).map_err(|_| "JSONDecodeError")?;
    let tools = pinned::parse_pinned_tools(decoded).map_err(|error| match error {
        pinned::PinnedError::Decode(_) => "JSONDecodeError",
        pinned::PinnedError::Validation(_) => "ValidationError",
    })?;
    serde_json::to_value(tools).map_err(|_| "SerializationError")
}

fn parse_metadata(value: &Value) -> Result<Value, &'static str> {
    let metadata: BTreeMap<String, JsonValue> =
        serde_json::from_value(value.clone()).map_err(|_| "ValidationError")?;
    let validated =
        pinned::validate_mcp_protocol_metadata(metadata).map_err(|_| "ValidationError")?;
    serde_json::to_value(validated).map_err(|_| "SerializationError")
}
