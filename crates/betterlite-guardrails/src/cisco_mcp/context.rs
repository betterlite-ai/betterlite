use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;

pub fn hydrate_tool_context(data: &mut Fields) {
    let Some(metadata) = tool_call_metadata(data) else {
        return;
    };
    set_if(
        data,
        &metadata,
        "name",
        &["mcp_tool_name", "tool_name", "name"],
        Json::truthy,
    );
    set_if(
        data,
        &metadata,
        "arguments",
        &["mcp_arguments", "arguments"],
        |value| !matches!(value, Json::Null),
    );
    set_if(
        data,
        &metadata,
        "mcp_server_name",
        &["mcp_server_name", "server_name"],
        Json::truthy,
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

fn set_if(
    data: &mut Fields,
    metadata: &Fields,
    source_key: &str,
    targets: &[&str],
    keep: impl Fn(&Json) -> bool,
) {
    let Some(raw) = metadata.get(source_key) else {
        return;
    };
    if !keep(&Json::parse(raw.get())) {
        return;
    }
    for target in targets {
        data.entry((*target).to_owned())
            .or_insert_with(|| raw.clone());
    }
}

mod tests;
