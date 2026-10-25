use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::Value;

use crate::options::HarnessOptions;
use crate::sandbox::Sandbox;
use crate::types::{Approval, Harness, PermissionMode};

pub type ApprovalHandler =
    Box<dyn Fn(&Approval) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;
pub struct ModelEndpoint;
pub struct GatewayTarget {
    pub api_base: String,
    pub api_key: SecretString,
}
pub struct SessionContext {
    pub harness: Harness,
    pub sandbox: Box<dyn Sandbox>,
    pub session_id: String,
    pub model: Option<String>,
    pub gateway: Option<GatewayTarget>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub endpoint: Option<ModelEndpoint>,
    pub instructions: Option<String>,
    pub tools: Vec<String>,
    pub skills: Vec<String>,
    pub disable_tools: Vec<String>,
    pub permissions: PermissionMode,
    pub on_approval: Option<ApprovalHandler>,
    pub output: Option<String>,
    pub max_turns: Option<u32>,
    pub timeout: Option<Duration>,
    pub metadata: BTreeMap<String, Value>,
    pub options: Option<HarnessOptions>,
    pub final_text: String,
    pub output_json: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: Decimal,
    pub calls: u64,
}
