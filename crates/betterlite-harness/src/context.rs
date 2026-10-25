use std::sync::Arc;
use std::time::Duration;

use futures_core::future::BoxFuture;
use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::events::Approval;
use crate::options::HarnessOptions;
use crate::types::{Harness, JsonObject, PermissionMode, Usage};

pub type ApprovalHandler = Arc<dyn Fn(Approval) -> BoxFuture<'static, bool> + Send + Sync>;

#[derive(Clone, Debug)]
pub struct GatewayTarget {
    pub api_base: String,
    pub api_key: SecretString,
}

pub struct SessionContext<S> {
    pub harness: Harness,
    pub sandbox: S,
    pub session_id: String,
    pub settings: SessionSettings,
    pub outcome: TurnOutcome,
}

#[derive(Clone, Default)]
pub struct SessionSettings {
    // the name the runtime gets, with the litellm_proxy/ prefix already stripped
    pub model: Option<String>,
    pub gateway: Option<GatewayTarget>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub instructions: Option<String>,
    pub skills: Vec<String>,
    pub disable_tools: Vec<String>,
    pub permissions: PermissionMode,
    pub on_approval: Option<ApprovalHandler>,
    pub output_schema: Option<JsonObject>,
    pub max_turns: Option<u32>,
    pub timeout: Option<Duration>,
    pub metadata: JsonObject,
    pub options: Option<HarnessOptions>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TurnOutcome {
    pub final_text: String,
    pub output_json: Option<String>,
    // in-process harnesses call the model themselves, so their usage lands here instead of at an endpoint
    pub usage: Usage,
    pub cost: Decimal,
}
