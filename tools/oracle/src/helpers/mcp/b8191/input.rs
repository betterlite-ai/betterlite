use std::collections::BTreeMap;

use betterlite_db::identity_rows::Payload;
use betterlite_mcp::request_context::allowlist::{McpClientAllowlist, McpClientAllowlistSettings};
use betterlite_mcp::request_context::elicitation::{ElicitRequestParams, ElicitResult};
use betterlite_mcp::wire_compat::capabilities::policy::ClientCapabilities;
use serde::Deserialize;
use serde::de::Error as _;
use serde_json::Value;

// an internally tagged enum buffers its payload where a RawValue cannot be read, so dispatch here
#[derive(Deserialize)]
#[serde(try_from = "Value")]
pub(crate) enum Input {
    AllowlistLoad(AllowlistLoadInput),
    AllowlistCheck(AllowlistCheckInput),
    Elicitation(ElicitationInput),
}

impl TryFrom<Value> for Input {
    type Error = serde_json::Error;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let operation = value
            .get("operation")
            .and_then(Value::as_str)
            .ok_or_else(|| serde_json::Error::custom("the helper input names no operation"))?;
        match operation {
            "allowlist_load" => Ok(Self::AllowlistLoad(serde_json::from_value(value)?)),
            "allowlist_check" => Ok(Self::AllowlistCheck(serde_json::from_value(value)?)),
            "elicitation" => Ok(Self::Elicitation(serde_json::from_value(value)?)),
            other => Err(serde_json::Error::custom(format!(
                "unknown b8191 operation: {other}"
            ))),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct AllowlistLoadInput {
    #[serde(default)]
    pub settings: McpClientAllowlistSettings,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AllowlistCheckInput {
    #[serde(default)]
    pub scenarios: Vec<CheckScenario>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CheckScenario {
    pub allowlist: Option<McpClientAllowlist>,
    pub jwt_claims: Option<Payload>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub(crate) struct ElicitationInput {
    #[serde(default)]
    pub scenarios: Vec<ElicitationScenario>,
}

#[derive(Deserialize)]
pub(crate) struct ElicitationScenario {
    pub params: ElicitRequestParams,
    pub capabilities: Option<ClientCapabilities>,
    pub session: Option<SessionBehaviour>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum SessionBehaviour {
    Result { result: ElicitResult },
    Error { error: String },
}
