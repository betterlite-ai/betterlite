use std::collections::BTreeMap;

use betterlite_core::api::responses_tool_namespaces::error::NamespaceToolsError;
use betterlite_core::api::responses_tool_namespaces::values::{Fields, text_field};

use super::as_tool::as_tool;

mod tests;

pub type IndexedKey = (String, usize);

// A chat tool is keyed by its function name, and any other tool by its server label, else its name.
pub fn chat_tool_key(tool: &Fields) -> Result<String, NamespaceToolsError> {
    let tool_type = text_field(tool, "type")?;
    if let Some(function) = tool.get("function").and_then(as_tool) {
        return Ok(format!("{tool_type}:{}", text_field(function, "name")?));
    }
    let server_label = text_field(tool, "server_label")?;
    let name = if server_label.is_empty() {
        text_field(tool, "name")?
    } else {
        server_label
    };
    Ok(format!("{tool_type}:{name}"))
}

// The nth tool with the same key gets ordinal n, so duplicate names stay apart.
pub fn indexed_keys(tools: &[&Fields]) -> Result<Vec<IndexedKey>, NamespaceToolsError> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut keys = Vec::with_capacity(tools.len());
    for tool in tools {
        let key = chat_tool_key(tool)?;
        let ordinal = seen.entry(key.clone()).or_default();
        keys.push((key, *ordinal));
        *ordinal = ordinal.saturating_add(1);
    }
    Ok(keys)
}
