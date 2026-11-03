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
    if let Some(raw) = data.get("mcp_tool_call_metadata") {
        let direct = Json::parse(raw.get());
        if matches!(direct, Json::Null) {
            return nested_tool_call_metadata(data);
        }
        return object_metadata(direct);
    }
    nested_tool_call_metadata(data)
}

fn nested_tool_call_metadata(data: &Fields) -> Option<Fields> {
    let metadata = data.get("metadata").map_or("null", |raw| raw.get());
    let selected = if Json::parse(metadata).truthy() {
        metadata
    } else {
        data.get("litellm_metadata").map_or("null", |raw| raw.get())
    };
    let Some(nested) = object_metadata(Json::parse(selected)) else {
        return None;
    };
    let raw = nested
        .get("mcp_tool_call_metadata")
        .map_or("null", |value| value.get());
    object_metadata(Json::parse(raw))
}

fn object_metadata(value: Json) -> Option<Fields> {
    if let Json::Object(metadata) = value {
        Some(metadata)
    } else {
        None
    }
}

fn set_if<Keep>(
    data: &mut Fields,
    metadata: &Fields,
    source_key: &str,
    targets: &[&str],
    keep: Keep,
) where
    Keep: Fn(&Json) -> bool,
{
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
