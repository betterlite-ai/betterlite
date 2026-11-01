use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;

pub fn hydrate_tool_context(data: &mut Fields) {
    let Some(metadata) = tool_call_metadata(data) else {
        return;
    };
    set_if_nonfalsy(
        data,
        &metadata,
        "name",
        &["mcp_tool_name", "tool_name", "name"],
    );
    set_if_present(
        data,
        &metadata,
        "arguments",
        &["mcp_arguments", "arguments"],
    );
    set_if_nonfalsy(
        data,
        &metadata,
        "mcp_server_name",
        &["mcp_server_name", "server_name"],
    );
}

fn tool_call_metadata(data: &Fields) -> Option<Fields> {
    if let Json::Object(metadata) = Json::field(data, "mcp_tool_call_metadata") {
        return Some(metadata);
    }
    for key in ["metadata", "litellm_metadata"] {
        let Json::Object(nested) = Json::field(data, key) else {
            continue;
        };
        if let Json::Object(metadata) = Json::field(&nested, "mcp_tool_call_metadata") {
            return Some(metadata);
        }
    }
    None
}

fn set_if_nonfalsy(data: &mut Fields, metadata: &Fields, source_key: &str, targets: &[&str]) {
    let Some(raw) = metadata.get(source_key) else {
        return;
    };
    if !Json::parse(raw.get()).truthy() {
        return;
    }
    for target in targets {
        data.entry((*target).to_owned())
            .or_insert_with(|| raw.clone());
    }
}

fn set_if_present(data: &mut Fields, metadata: &Fields, source_key: &str, targets: &[&str]) {
    let Some(raw) = metadata.get(source_key) else {
        return;
    };
    if matches!(Json::parse(raw.get()), Json::Null) {
        return;
    }
    for target in targets {
        data.entry((*target).to_owned())
            .or_insert_with(|| raw.clone());
    }
}

mod tests;
