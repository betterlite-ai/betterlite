use crate::gemini_tool_invokes::types::GeminiTool;

pub const THOUGHT_SIGNATURE_SEPARATOR: &str = "__thought__";
// base64 of b"skip_thought_signature_validator", the placeholder Google documents as a last resort
pub const DUMMY_THOUGHT_SIGNATURE: &str = "c2tpcF90aG91Z2h0X3NpZ25hdHVyZV92YWxpZGF0b3I=";

#[must_use]
pub fn thought_signature_from_tool(tool: &GeminiTool) -> Option<String> {
    let tool_signature = tool
        .provider_specific_fields
        .as_ref()
        .and_then(|fields| fields.thought_signature.as_deref())
        .filter(|signature| !signature.is_empty());
    if let Some(signature) = tool_signature {
        return Some(signature.to_owned());
    }
    let function_signature = tool
        .function
        .as_ref()
        .and_then(|function| function.provider_specific_fields.as_ref())
        .and_then(|fields| fields.thought_signature.as_deref())
        .filter(|signature| !signature.is_empty());
    if let Some(signature) = function_signature {
        return Some(signature.to_owned());
    }
    let (_, signature) = tool
        .id
        .as_deref()?
        .split_once(THOUGHT_SIGNATURE_SEPARATOR)?;
    if signature.is_empty() {
        return None;
    }
    Some(signature.to_owned())
}

#[must_use]
pub fn encode_tool_call_id(tool_call_id: &str, thought_signature: Option<&str>) -> String {
    match thought_signature {
        Some(signature) if !signature.is_empty() => {
            format!("{tool_call_id}{THOUGHT_SIGNATURE_SEPARATOR}{signature}")
        }
        Some(_) | None => tool_call_id.to_owned(),
    }
}
