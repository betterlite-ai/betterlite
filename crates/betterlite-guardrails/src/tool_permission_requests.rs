use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_selection::gate::GuardrailSelection as _;
use crate::tool_permission::ToolPermissionGuardrail;
use block_mode::denied_tool_names;
use early_return::guardrail_base;
use extraction::extract_request_tools;
use header_mutation::record_applied_guardrail;
use rewrite_mode::rewrite_forbidden_tools;

pub mod allowed_tools;
pub mod block_mode;
pub mod early_return;
pub mod error;
pub mod extraction;
pub mod header_mutation;
pub mod multiple_forbidden;
pub mod permission_check;
pub mod rewrite_mode;
mod tests;

type Data = BTreeMap<String, GuardrailValue>;

// LiteLLM's async_pre_call_hook: `suppressed` is what an auto router turned off for this request
pub fn pre_call_hook(
    guardrail: &ToolPermissionGuardrail,
    data: &mut Data,
    suppressed: &BTreeSet<String>,
) -> Result<(), GuardrailFailure> {
    tracing::debug!("Tool Permission Guardrail Pre-Call Hook");
    let base = guardrail_base(guardrail);
    if !base.should_run_guardrail(data, GuardrailEventHooks::PreCall, suppressed)? {
        return Ok(());
    }
    let extracted = extract_request_tools(data)?;
    let names = extracted.names();
    if names.is_empty() {
        tracing::debug!(
            "Tool Permission Guardrail: not running guardrail. No tools or functions in data"
        );
        return Ok(());
    }
    let denied = denied_tool_names(guardrail, names)?;
    rewrite_forbidden_tools(data, &extracted, &denied);
    tracing::debug!("Tool Permission Guardrail Pre-Call Hook: All tools allowed");
    record_applied_guardrail(data, &guardrail.args.name);
    Ok(())
}
