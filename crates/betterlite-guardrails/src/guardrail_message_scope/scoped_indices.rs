use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

mod tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GuardrailSkipFlags {
    pub skip_system_message: Option<bool>,
    pub skip_tool_message: Option<bool>,
    pub scan_only_tool_results: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GlobalSkipFlags {
    pub skip_system_message: bool,
    pub skip_tool_message: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ScanScope {
    #[default]
    AllMessages,
    ToolResultsOnly,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScopeFlags {
    pub skip_system_message: bool,
    pub skip_tool_message: bool,
    pub scan: ScanScope,
}

impl ScopeFlags {
    // A guardrail's own flag wins; a flag it leaves unset falls back to the global one, and unset everywhere means scan.
    #[must_use]
    pub fn resolve(guardrail: &GuardrailSkipFlags, global: &GlobalSkipFlags) -> Self {
        Self {
            skip_system_message: guardrail
                .skip_system_message
                .unwrap_or(global.skip_system_message),
            skip_tool_message: guardrail
                .skip_tool_message
                .unwrap_or(global.skip_tool_message),
            scan: if guardrail.scan_only_tool_results == Some(true) {
                ScanScope::ToolResultsOnly
            } else {
                ScanScope::AllMessages
            },
        }
    }
}

#[must_use]
pub fn message_role(message: &GuardrailValue) -> String {
    let GuardrailValue::Object(fields) = message else {
        return String::new();
    };
    let Some(GuardrailValue::Text(role)) = fields.get("role") else {
        return String::new();
    };
    role.to_lowercase()
}

#[must_use]
pub fn role_out_of_guardrail_scope(role: &str, flags: &ScopeFlags) -> bool {
    if flags.skip_system_message && role == "system" {
        return true;
    }
    if flags.skip_tool_message && role == "tool" {
        return true;
    }
    flags.scan == ScanScope::ToolResultsOnly && role != "tool" && role != "function"
}

#[must_use]
pub fn scoped_message_indices(messages: &[GuardrailValue], flags: &ScopeFlags) -> Vec<usize> {
    messages
        .iter()
        .enumerate()
        .filter(|(_, message)| !role_out_of_guardrail_scope(&message_role(message), flags))
        .map(|(index, _)| index)
        .collect()
}

// Only the two skip flags filter here; scan-only scope never drops a row from this list.
#[must_use]
pub fn filter_messages_by_skip_flags(
    messages: &[GuardrailValue],
    flags: &ScopeFlags,
) -> (Vec<GuardrailValue>, bool) {
    let kept: Vec<GuardrailValue> = messages
        .iter()
        .filter(|message| {
            let role = message_role(message);
            (!flags.skip_system_message || role != "system")
                && (!flags.skip_tool_message || role != "tool")
        })
        .cloned()
        .collect();
    let changed = kept.len() != messages.len();
    (kept, changed)
}
