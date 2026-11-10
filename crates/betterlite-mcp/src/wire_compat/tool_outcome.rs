use betterlite_prompts::arguments::json::Json;
use serde::{Deserialize, Serialize};

use crate::client::tools::result::{CallToolResult, InputRequiredResult};

use super::capabilities::versions::SpecVersion;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WireCompat {
    Legacy,
    Modern,
}

#[must_use]
pub fn wire_compat_for(protocol_version: &str) -> WireCompat {
    if SpecVersion::from_wire(protocol_version).is_some_and(|version| !version.is_handshake()) {
        WireCompat::Modern
    } else {
        WireCompat::Legacy
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextResult {
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonResult {
    pub value: Json,
    pub original_text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolFailure {
    pub error_type: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolOutcome {
    Text(TextResult),
    Json(JsonResult),
    Complete(CallToolResult),
    InputRequired(InputRequiredResult),
    Failure(ToolFailure),
}

#[must_use]
pub fn parse_http_body(body: &str) -> ToolOutcome {
    match serde_json::from_str::<Json>(body) {
        Ok(Json::Null) | Err(_) => ToolOutcome::Text(TextResult {
            text: body.to_owned(),
        }),
        Ok(value) => ToolOutcome::Json(JsonResult {
            value,
            original_text: body.to_owned(),
        }),
    }
}

mod tests;
