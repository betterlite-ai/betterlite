use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::enums::Role;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::LakeraHookError;

const SYSTEM: &str = "system";
const USER: &str = "user";
const ASSISTANT: &str = "assistant";
const TOOL_CALLS: &str = "messages.tool_calls";
const FUNCTION: &str = "messages.tool_calls.function";
const ARGUMENTS: &str = "messages.tool_calls.function.arguments";
const CONTENT: &str = "messages.content";

pub(super) enum CheckBody {
    Skip,
    Empty,
    Json(GuardrailValue),
}

// one entry per role in INPUT_POSITIONING_MAP order, a later message of a role replacing the earlier one
pub(super) fn messages_body(
    messages: &[GuardrailValue],
    enabled: &[Role],
    add_function_to_prompt: bool,
    dynamic: &BTreeMap<String, GuardrailValue>,
) -> Result<CheckBody, LakeraHookError> {
    let mut roles = Roles::default();
    roles.fill(messages, enabled)?;
    let mut inspection = Vec::new();
    if let Some(content) = roles.system
        && !add_function_to_prompt
    {
        inspection.push(system_entry(content, &roles.arguments)?);
    }
    inspection.extend(roles.user);
    inspection.extend(roles.assistant);
    if inspection.is_empty() {
        tracing::debug!("Skipping lakera prompt injection, no roles with messages found");
        return Ok(CheckBody::Skip);
    }
    let mut body = BTreeMap::from([("input".to_owned(), GuardrailValue::Array(inspection))]);
    // LiteLLM spreads the dynamic params into the body, where they win over the built keys
    body.extend(dynamic.clone());
    Ok(CheckBody::Json(GuardrailValue::Object(body)))
}

#[derive(Default)]
struct Roles {
    system: Option<GuardrailValue>,
    user: Option<GuardrailValue>,
    assistant: Option<GuardrailValue>,
    arguments: Vec<String>,
}

impl Roles {
    fn fill(
        &mut self,
        messages: &[GuardrailValue],
        enabled: &[Role],
    ) -> Result<(), LakeraHookError> {
        for message in messages {
            let GuardrailValue::Object(fields) = message else {
                return Err(LakeraHookError::InvalidField { field: "messages" });
            };
            let Some(role) = enabled_role(fields.get("role"), enabled) else {
                continue;
            };
            if let Some(calls) = fields
                .get("tool_calls")
                .filter(|value| !matches!(value, GuardrailValue::Null))
            {
                collect_function_arguments(calls, &mut self.arguments)?;
            }
            self.keep(role, fields.get("content"));
        }
        Ok(())
    }

    fn keep(&mut self, role: Role, content: Option<&GuardrailValue>) {
        match role {
            Role::System => self.system = Some(content.cloned().unwrap_or(GuardrailValue::Null)),
            Role::User => self.user = Some(entry(USER, content)),
            Role::Assistant => self.assistant = Some(entry(ASSISTANT, content)),
        }
    }
}

const fn role_name(role: Role) -> &'static str {
    match role {
        Role::System => SYSTEM,
        Role::User => USER,
        Role::Assistant => ASSISTANT,
    }
}

fn enabled_role(value: Option<&GuardrailValue>, roles: &[Role]) -> Option<Role> {
    let GuardrailValue::Text(name) = value? else {
        return None;
    };
    roles
        .iter()
        .copied()
        .find(|role| role_name(*role) == name.as_str())
}

fn entry(role: &str, content: Option<&GuardrailValue>) -> GuardrailValue {
    let value = content.cloned().unwrap_or(GuardrailValue::Null);
    GuardrailValue::Object(BTreeMap::from([
        ("role".to_owned(), GuardrailValue::Text(role.to_owned())),
        ("content".to_owned(), value),
    ]))
}

fn collect_function_arguments(
    tool_calls: &GuardrailValue,
    arguments: &mut Vec<String>,
) -> Result<(), LakeraHookError> {
    let GuardrailValue::Array(calls) = tool_calls else {
        return Err(LakeraHookError::InvalidField { field: TOOL_CALLS });
    };
    for item in calls {
        let GuardrailValue::Object(call) = item else {
            return Err(LakeraHookError::InvalidField { field: TOOL_CALLS });
        };
        match call.get("function") {
            None | Some(GuardrailValue::Null) => {}
            Some(GuardrailValue::Object(function)) => match function.get("arguments") {
                None | Some(GuardrailValue::Null) => {}
                Some(GuardrailValue::Text(text)) => arguments.push(text.clone()),
                Some(_) => return Err(LakeraHookError::InvalidField { field: ARGUMENTS }),
            },
            Some(_) => return Err(LakeraHookError::InvalidField { field: FUNCTION }),
        }
    }
    Ok(())
}

fn system_entry(
    content: GuardrailValue,
    arguments: &[String],
) -> Result<GuardrailValue, LakeraHookError> {
    if arguments.is_empty() {
        return Ok(entry(SYSTEM, Some(&content)));
    }
    let GuardrailValue::Text(mut text) = content else {
        return Err(LakeraHookError::InvalidField { field: CONTENT });
    };
    text.push_str(" Function Input: ");
    text.push_str(&arguments.join(" "));
    Ok(entry(SYSTEM, Some(&GuardrailValue::Text(text))))
}

pub(super) fn join_input_texts(items: &[GuardrailValue]) -> Result<String, LakeraHookError> {
    let mut texts = Vec::with_capacity(items.len());
    for item in items {
        let GuardrailValue::Text(text) = item else {
            return Err(LakeraHookError::InvalidField { field: "input" });
        };
        texts.push(text.clone());
    }
    Ok(texts.join("\n"))
}

pub(super) fn input_body(text: &str, dynamic: &BTreeMap<String, GuardrailValue>) -> GuardrailValue {
    let mut body = BTreeMap::from([("input".to_owned(), GuardrailValue::Text(text.to_owned()))]);
    body.extend(dynamic.clone());
    GuardrailValue::Object(body)
}
