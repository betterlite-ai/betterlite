use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

pub(super) const MCP_CALL_TYPE: &str = "call_mcp_tool";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HookPhase {
    PreCall,
    Moderation,
}

impl HookPhase {
    // LiteLLM translates an MCP tool call before it asks whether the guardrail should run
    pub(super) fn event_type(self, call_type: Option<&str>) -> GuardrailEventHooks {
        let mcp = call_type.is_some_and(|kind| kind == MCP_CALL_TYPE);
        match (self, mcp) {
            (Self::PreCall, false) => GuardrailEventHooks::PreCall,
            (Self::PreCall, true) => GuardrailEventHooks::PreMcpCall,
            (Self::Moderation, false) => GuardrailEventHooks::DuringCall,
            (Self::Moderation, true) => GuardrailEventHooks::DuringMcpCall,
        }
    }

    pub(super) const fn hook_name(self) -> &'static str {
        match self {
            Self::PreCall => "pre-call",
            Self::Moderation => "moderation",
        }
    }
}

mod tests;
