use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_redact::structured::SensitiveDataMasker;
use indexmap::IndexMap;

use super::REDACTED_BY_LITELM;
use super::truncate::truncate_text;
use super::value::{NodeId, SpendMapping, SpendValue, to_json};

const SECRET_FIELDS: &str = "secret_fields";
const TOOL_INPUT_BLOCK_TYPES: [&str; 3] = ["tool_use", "server_tool_use", "mcp_tool_use"];
const TOOL_OUTPUT_BLOCK_TYPES: [&str; 3] =
    ["tool_result", "mcp_tool_result", "function_call_output"];

#[must_use]
pub fn sanitize_request_body(body: &SpendMapping, max_string_length: i64) -> Json {
    // LiteLLM adds "apikey" to the default credential patterns for request bodies only
    let masker = SensitiveDataMasker::with_patterns(["apikey"], Vec::<String>::new());
    let mut sanitizer = Sanitizer {
        visited: BTreeSet::new(),
        max_string_length,
        masker: &masker,
    };
    sanitizer.mapping(body, true)
}

struct Sanitizer<'masker> {
    visited: BTreeSet<NodeId>,
    max_string_length: i64,
    masker: &'masker SensitiveDataMasker,
}

impl Sanitizer<'_> {
    // LiteLLM tracks the mappings it has entered, so a repeated one becomes an empty object
    fn mapping(&mut self, mapping: &SpendMapping, mask_credentials: bool) -> Json {
        if !self.visited.insert(mapping.node) {
            return Json::Object(IndexMap::new());
        }
        let mut sanitized = IndexMap::new();
        for (key, value) in &mapping.entries {
            if key == SECRET_FIELDS {
                continue;
            }
            if mask_credentials
                && value.as_text().is_some()
                && self.masker.is_sensitive_key(key, &[])
            {
                sanitized.insert(key.clone(), Json::Text(REDACTED_BY_LITELM.to_owned()));
                continue;
            }
            let nested_mask = mask_credentials && !is_spend_log_content(mapping, key);
            let item = self.value(value, nested_mask);
            sanitized.insert(key.clone(), item);
        }
        Json::Object(sanitized)
    }

    fn value(&mut self, value: &SpendValue, mask_credentials: bool) -> Json {
        match value {
            SpendValue::Mapping(mapping) => self.mapping(mapping, mask_credentials),
            SpendValue::List(list) => Json::List(
                list.items
                    .iter()
                    .map(|item| self.value(item, mask_credentials))
                    .collect(),
            ),
            SpendValue::Text(text) => Json::Text(truncate_text(text, self.max_string_length)),
            SpendValue::Null
            | SpendValue::Bool(_)
            | SpendValue::Number(_)
            | SpendValue::Model(_)
            | SpendValue::Object(_) => to_json(value),
        }
    }
}

fn is_spend_log_content(parent: &SpendMapping, key: &str) -> bool {
    let block_type = parent.entries.get("type").and_then(SpendValue::as_text);
    match key {
        "arguments" | "logprobs" => true,
        "input" => block_type.is_some_and(|name| TOOL_INPUT_BLOCK_TYPES.contains(&name)),
        "content" | "output" => {
            block_type.is_some_and(|name| TOOL_OUTPUT_BLOCK_TYPES.contains(&name))
                || parent.entries.get("role").and_then(SpendValue::as_text) == Some("tool")
        }
        _ => false,
    }
}
