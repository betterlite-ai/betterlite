use std::sync::Arc;

use betterlite_pyhost::blocking_work::WorkTracker;
use betterlite_pyhost::hook_object::HookObject;
use betterlite_pyhost::host::PythonHost;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKey {
    Callbacks,
    SuccessCallback,
    FailureCallback,
    AuditLogCallbacks,
}

impl ListKey {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Callbacks => "callbacks",
            Self::SuccessCallback => "success_callback",
            Self::FailureCallback => "failure_callback",
            Self::AuditLogCallbacks => "audit_log_callbacks",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookSlot {
    CustomAuth,
    CustomKeyGenerate,
    CustomKeyUpdate,
    CustomKeyPolicy,
    CustomTeamMetadataValidate,
    CustomSso,
    CustomUiSsoSignInHandler,
    PostCallRules,
    CustomValidate,
    PassThroughAdapter,
    McpToolHandler,
    GuardrailClass,
    CustomProvider,
    RoutingPlugin,
    ClassifierPlugin,
    Callback(ListKey),
}

impl HookSlot {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CustomAuth => "custom_auth",
            Self::CustomKeyGenerate => "custom_key_generate",
            Self::CustomKeyUpdate => "custom_key_update",
            Self::CustomKeyPolicy => "custom_key_policy",
            Self::CustomTeamMetadataValidate => "custom_team_metadata_validate",
            Self::CustomSso => "custom_sso",
            Self::CustomUiSsoSignInHandler => "custom_ui_sso_sign_in_handler",
            Self::PostCallRules => "post_call_rules",
            Self::CustomValidate => "custom_validate",
            Self::PassThroughAdapter => "pass_through_adapter",
            Self::McpToolHandler => "mcp_tool_handler",
            Self::GuardrailClass => "guardrail_class",
            Self::CustomProvider => "custom_provider",
            Self::RoutingPlugin => "routing_plugin",
            Self::ClassifierPlugin => "classifier_plugin",
            Self::Callback(list) => list.as_str(),
        }
    }
}

pub struct LoadedHook {
    pub slot: HookSlot,
    pub value: String,
    pub object: HookObject,
    pub host: PythonHost,
    pub tracker: Arc<WorkTracker>,
}

pub enum LoadedCallback {
    Named(String),
    Loaded(LoadedHook),
}

mod tests;
