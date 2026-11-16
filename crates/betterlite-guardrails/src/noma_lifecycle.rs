pub mod config;
pub mod error;
pub mod hooks;
pub mod initializer;
pub mod stream_error;
pub mod streaming;

mod background;
mod failure;
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use tokio_util::task::TaskTracker;

use crate::guardrail_base::GuardrailBase;
use crate::noma_scan_policy::NomaScanPolicy;
use crate::noma_scan_policy::config::NomaGuardrailConfigModel;
use crate::noma_v2::guardrail::NomaV2Guardrail;

pub type RequestData = BTreeMap<String, GuardrailValue>;

// LiteLLM's get_config_model returns this class
pub type ConfigModel = NomaGuardrailConfigModel;

pub const MCP_CALL_TYPE: &str = "call_mcp_tool";

pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 4] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::PreMcpCall,
];

// LiteLLM's module-level _LEGACY_NOMA_DEPRECATION_WARNED: the warning is per process, not per instance
static LEGACY_NOMA_DEPRECATION_WARNED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone)]
pub struct NomaGuardrail {
    pub base: GuardrailBase,
    pub policy: NomaScanPolicy,
    pub block_failures: bool,
    // monitor mode checks run here, so the server can drain them on shutdown
    pub tasks: TaskTracker,
}

// What the initializers and the class registry hand back: the legacy guardrail, or the v2 one `use_v2` routes to
#[derive(Debug, Clone)]
pub enum NomaCallback {
    Legacy(Arc<NomaGuardrail>),
    V2(Arc<NomaV2Guardrail>),
}
