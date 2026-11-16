use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::{
    JsonValue, MCPAuthType, MCPCredentials, MCPTokenEndpointAuthMethod, MCPTransport, decode,
    pinned,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::{Deserialize, Serialize};
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

impl Call {
    #[expect(
        clippy::map_err_ignore,
        reason = "the step answer is only the exception class LiteLLM raises, so each failure keeps its class and drops the error"
    )]
    fn evaluate(&self) -> Result<Value, &'static str> {
        match self {
            Self::CredentialContract(value) => {
                let record = serde_json::from_value::<MCPCredentials>(value.clone())
                    .map_err(|_| "ValidationError")?;
                serde_json::to_value(record).map_err(|_| "SerializationError")
            }
            Self::Transport(value) => {
                let record = serde_json::from_value::<MCPTransport>(value.clone())
                    .map_err(|_| "ValidationError")?;
                serde_json::to_value(record).map_err(|_| "SerializationError")
            }
            Self::Auth(value) => {
                let record = serde_json::from_value::<MCPAuthType>(value.clone())
                    .map_err(|_| "ValidationError")?;
                serde_json::to_value(record).map_err(|_| "SerializationError")
            }
            Self::TokenEndpointMethod(value) => {
                let record = serde_json::from_value::<MCPTokenEndpointAuthMethod>(value.clone())
                    .map_err(|_| "ValidationError")?;
                serde_json::to_value(record).map_err(|_| "SerializationError")
            }
            Self::PinnedToolContract(value) => parse_tools(value),
            Self::ProtocolMetadata(value) => parse_metadata(value),
        }
    }
}

#[derive(Serialize)]
struct Outcome {
    value: Option<Value>,
    error: Option<&'static str>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let results: BTreeMap<_, _> = input
            .steps
            .iter()
            .map(|(name, call)| {
                let result = call.evaluate();
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
            Box::new(HelperError {
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

#[expect(
    clippy::map_err_ignore,
    reason = "the step answer is only the exception class LiteLLM raises, so each failure keeps its class and drops the error"
)]
fn parse_tools(value: &Value) -> Result<Value, &'static str> {
    let bytes = serde_json::to_vec(value).map_err(|_| "SerializationError")?;
    let decoded = decode::decode_mcp_stored_contract(&bytes).map_err(|_| "JSONDecodeError")?;
    let tools = pinned::parse_pinned_tools(decoded).map_err(|error| match error {
        pinned::PinnedError::Decode(_) => "JSONDecodeError",
        pinned::PinnedError::Validation(_) | pinned::PinnedError::InvalidToolRecord => {
            "ValidationError"
        }
    })?;
    serde_json::to_value(tools).map_err(|_| "SerializationError")
}

#[expect(
    clippy::map_err_ignore,
    reason = "the step answer is only the exception class LiteLLM raises, so each failure keeps its class and drops the error"
)]
fn parse_metadata(value: &Value) -> Result<Value, &'static str> {
    let metadata: BTreeMap<String, JsonValue> =
        serde_json::from_value(value.clone()).map_err(|_| "ValidationError")?;
    let validated =
        pinned::validate_mcp_protocol_metadata(metadata).map_err(|_| "ValidationError")?;
    serde_json::to_value(validated).map_err(|_| "SerializationError")
}
