use crate::arguments::json::Json;

const NATIVE_KEYS: [&str; 3] = ["systemTool", "toolSpec", "cachePoint"];

#[must_use]
pub fn is_bedrock_tool_block(tool: &Json) -> bool {
    let Json::Object(fields) = tool else {
        return false;
    };
    NATIVE_KEYS.iter().any(|key| fields.contains_key(*key))
}
