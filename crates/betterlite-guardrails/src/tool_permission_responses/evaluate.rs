use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_initializers::tool_permission::ToolDisallowedAction;
use crate::tool_permission::ToolPermissionGuardrail;
use crate::tool_permission::tool_permission_evaluation::{
    ToolCallPermission, get_permission_for_tool_call,
};
use crate::tool_permission::types::PermissionError;

use super::extraction::ToolCallInfo;

// One denied call and the permission error built for it, in the order the calls were extracted
#[derive(Debug, Clone, PartialEq)]
pub struct DeniedToolCall {
    pub call: ToolCallInfo,
    pub error: PermissionError,
}

// LiteLLM's _evaluate_tool_calls: every call is checked first, then the first denial raises in block mode
pub fn evaluate_tool_calls(
    guardrail: &ToolPermissionGuardrail,
    calls: &[ToolCallInfo],
) -> Result<Vec<DeniedToolCall>, GuardrailFailure> {
    let checked: Vec<(ToolCallInfo, ToolCallPermission)> = calls
        .iter()
        .map(|call| {
            let name = call.name.as_deref();
            let kind = call.kind.as_deref();
            let arguments = call.arguments.as_ref();
            let permission = get_permission_for_tool_call(guardrail, name, kind, arguments);
            (call.clone(), permission)
        })
        .collect();
    for (_call, permission) in &checked {
        let Some(message) = &permission.message else {
            continue;
        };
        if permission.is_allowed {
            continue;
        }
        tracing::info!("Tool Permission Guardrail: {message}");
        if guardrail.args.on_disallowed_action == ToolDisallowedAction::Block {
            return Err(GuardrailFailure::GuardrailRaised {
                guardrail_name: Some(guardrail.args.name.clone()),
                message: message.clone(),
                wrap_default_message: true,
                status_code: 400,
                blocked_content: true,
            });
        }
    }
    let denied = checked.into_iter().filter_map(denial).collect();
    Ok(denied)
}

// The denied half of the checked calls, each with the error its message and rule id build
fn denial((call, permission): (ToolCallInfo, ToolCallPermission)) -> Option<DeniedToolCall> {
    if permission.is_allowed {
        return None;
    }
    let message = permission.message?;
    Some(DeniedToolCall {
        error: PermissionError {
            tool_name: call
                .name
                .clone()
                .unwrap_or_else(|| "unknown_tool".to_owned()),
            rule_id: permission.rule_id,
            message,
        },
        call,
    })
}

mod tests;
