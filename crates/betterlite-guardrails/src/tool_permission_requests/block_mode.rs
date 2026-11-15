use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;

use super::permission_check::check_tool_permission;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_initializers::tool_permission::ToolDisallowedAction;
use crate::tool_permission::ToolPermissionGuardrail;

mod tests;

// Block mode fails on the first forbidden tool; rewrite mode collects every forbidden name
pub fn denied_tool_names<'request>(
    guardrail: &ToolPermissionGuardrail,
    names: Vec<&'request str>,
) -> Result<Vec<&'request str>, GuardrailFailure> {
    let mut denied = Vec::new();
    for name in names {
        let verdict = check_tool_permission(guardrail, name);
        if verdict.is_allowed {
            continue;
        }
        tracing::info!("Tool Permission Guardrail: {}", verdict.message);
        match guardrail.args.on_disallowed_action {
            ToolDisallowedAction::Block => {
                return Err(GuardrailFailure::HttpObject {
                    status: 400,
                    detail: IndexMap::from([
                        (
                            "error".to_owned(),
                            GuardrailValue::Text("Violated guardrail policy".to_owned()),
                        ),
                        (
                            "detection_message".to_owned(),
                            GuardrailValue::Text(verdict.message),
                        ),
                    ]),
                });
            }
            ToolDisallowedAction::Rewrite => denied.push(name),
        }
    }
    Ok(denied)
}
