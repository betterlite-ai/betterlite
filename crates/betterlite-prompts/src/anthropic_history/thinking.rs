use serde_json::Value;

pub fn drop_unsignable(blocks: &[Value]) -> Vec<Value> {
    blocks
        .iter()
        .filter(|block| !is_unsignable_thinking_block(block))
        .cloned()
        .collect()
}

fn is_unsignable_thinking_block(block: &Value) -> bool {
    let Some(object) = block.as_object() else {
        return false;
    };
    let field = match object.get("type").and_then(Value::as_str) {
        Some("thinking") => object.get("signature"),
        Some("redacted_thinking") => object.get("data"),
        _ => None,
    };
    if field
        .and_then(Value::as_str)
        .is_some_and(|signature| signature.starts_with("litellm_encrypted_reasoning:"))
    {
        return true;
    }
    if object.get("type").and_then(Value::as_str) != Some("thinking") {
        return false;
    }
    let signature = object.get("signature").and_then(Value::as_str);
    let thinking = object.get("thinking").and_then(Value::as_str);
    !signature.is_some_and(|signature| !signature.is_empty())
        || !thinking.is_some_and(|thinking| !thinking.trim().is_empty())
}
