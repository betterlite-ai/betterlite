pub mod compat;
pub mod core;
pub mod response;

use betterlite_core::agents::json::AgentJson;
use serde::{Deserialize, Serialize};

pub const AGENT_CARD_PATH_PARAM: &str = "agent_card_path";
pub const JSONRPC_VERSION: &str = "2.0";
pub const A2A_VERSION_HEADER: &str = "A2A-Version";
pub const A2A_VERSION: &str = "1.0";
pub const SEND_MESSAGE_METHOD: &str = "SendMessage";
pub const SEND_STREAMING_MESSAGE_METHOD: &str = "SendStreamingMessage";

// Both protocol versions carry these message fields under the same names, so they pass through unchanged
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")] #[rustfmt::skip]
pub struct MessageCommon { #[serde(default)] pub message_id: String, #[serde(default, skip_serializing_if = "Option::is_none")] pub context_id: Option<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub task_id: Option<String>, #[serde(default, skip_serializing_if = "Option::is_none")] pub reference_task_ids: Option<Vec<String>>, #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<AgentJson>, #[serde(default, skip_serializing_if = "Option::is_none")] pub extensions: Option<Vec<String>> }
