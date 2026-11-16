use std::collections::BTreeMap;

use betterlite_core::api::responses_tool_namespaces::values::Fields;

use super::indexed_key::IndexedKey;

pub const DESCRIPTION_SEPARATOR: &str = "\n\n";
pub const CHAT_MEMBER_TYPES: [&str; 2] = ["function", "custom"];

// One original tool's flattened chat tools, their indexed keys and the guardrailed tools by key.
pub struct FlattenedGroup<'group> {
    pub tools: &'group [Fields],
    pub keys: &'group [IndexedKey],
    pub guardrailed: &'group BTreeMap<IndexedKey, &'group Fields>,
}
