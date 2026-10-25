use std::collections::BTreeMap;

use serde_json::Value;

use crate::options::HarnessOptions;
use crate::sandbox::Sandbox;
use crate::types::{Approval, Harness};

pub type ApprovalHandler = Box<dyn Fn(&Approval) -> bool + Send + Sync>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayTarget {
    pub api_base: String,
    pub api_key: String,
}

pub struct SessionContext {
    pub harness: Harness,
    pub sandbox: Box<dyn Sandbox>,
    pub session_id: String,
    pub model: Option<String>,
    pub gateway: Option<GatewayTarget>,
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub instructions: Option<String>,
    pub tools: Vec<String>,
    pub skills: Vec<String>,
    pub disable_tools: Vec<String>,
    pub permissions: String,
    pub on_approval: Option<ApprovalHandler>,
    pub output: Option<String>,
    pub max_turns: Option<u32>,
    pub timeout_seconds: Option<f64>,
    pub metadata: BTreeMap<String, Value>,
    pub options: Option<HarnessOptions>,
    pub final_text: String,
    pub output_json: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: f64,
    pub calls: u64,
}
