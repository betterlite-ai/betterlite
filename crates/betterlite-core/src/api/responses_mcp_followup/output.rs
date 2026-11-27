use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};
use betterlite_openai::api::responses_shapes::{GenericResponseOutputItem, OutputText};
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::responses_mcp_results::McpToolResult;

use super::records::IdSource;
use super::short_id;

const TOOLS_FETCHED_TYPE: &str = "mcp_tools_fetched";
const TOOLS_FETCHED_PREFIX: &str = "mcp_tools_";
const TOOL_RESULTS_TYPE: &str = "tool_execution_results";
const TOOL_RESULTS_PREFIX: &str = "tool_results_";
const COMPLETED_STATUS: &str = "completed";
const SYSTEM_ROLE: &str = "system";
const OUTPUT_TEXT_TYPE: &str = "output_text";

// _add_mcp_output_elements_to_response (L1310-L1357): the supplied response grows by fetched tools, then tool results
pub fn add_mcp_output_elements_to_response(
    response: &mut ResponsesApiResponse,
    mcp_tools_fetched: &[Json],
    tool_results: &[McpToolResult],
    ids: &IdSource,
) {
    let fetched_text = dumps(&Json::List(mcp_tools_fetched.to_vec()));
    let results_text = dumps(&tool_results_value(tool_results));
    let fetched = output_item(TOOLS_FETCHED_TYPE, TOOLS_FETCHED_PREFIX, &fetched_text, ids);
    let results = output_item(TOOL_RESULTS_TYPE, TOOL_RESULTS_PREFIX, &results_text, ids);
    response.output.push(ResponseOutputItem::Message(fetched));
    response.output.push(ResponseOutputItem::Message(results));
}

fn output_item(kind: &str, prefix: &str, text: &str, ids: &IdSource) -> GenericResponseOutputItem {
    GenericResponseOutputItem {
        r#type: kind.to_owned(),
        id: format!("{prefix}{}", short_id(ids)),
        status: COMPLETED_STATUS.to_owned(),
        role: SYSTEM_ROLE.to_owned(),
        content: vec![OutputText {
            r#type: Some(OUTPUT_TEXT_TYPE.to_owned()),
            text: Some(text.to_owned()),
            annotations: Some(Vec::new()),
            extra: BTreeMap::new(),
        }],
        phase: None,
        extra: BTreeMap::new(),
    }
}

// the producer's ToolResult dicts as json.dumps receives them, keys in tool_call_id, result, name order
fn tool_results_value(tool_results: &[McpToolResult]) -> Json {
    Json::List(
        tool_results
            .iter()
            .map(|result| {
                Json::Object(IndexMap::from([
                    (
                        String::from("tool_call_id"),
                        result
                            .tool_call_id
                            .as_ref()
                            .map_or(Json::Null, |id| Json::Text(id.clone())),
                    ),
                    (String::from("result"), Json::Text(result.result.clone())),
                    (
                        String::from("name"),
                        result
                            .name
                            .as_ref()
                            .map_or(Json::Null, |name| Json::Text(name.clone())),
                    ),
                ]))
            })
            .collect(),
    )
}

// json.dumps(value, indent=2, default=str), which on a decoded Json value is indent=2 with ensure_ascii escapes
fn dumps(value: &Json) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0);
    out
}

fn write_value(out: &mut String, value: &Json, depth: usize) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Json::Number(number) => out.push_str(&python_number(number)),
        Json::Text(text) => write_text(out, text),
        Json::List(items) => write_list(out, items, depth),
        Json::Object(fields) => write_object(out, fields, depth),
    }
}

fn write_list(out: &mut String, items: &[Json], depth: usize) {
    if items.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push_str("[\n");
    let inner = depth.saturating_add(1);
    for (position, item) in items.iter().enumerate() {
        if position != 0 {
            out.push_str(",\n");
        }
        write_indent(out, inner);
        write_value(out, item, inner);
    }
    out.push('\n');
    write_indent(out, depth);
    out.push(']');
}

fn write_object(out: &mut String, fields: &IndexMap<String, Json>, depth: usize) {
    if fields.is_empty() {
        out.push_str("{}");
        return;
    }
    out.push_str("{\n");
    let inner = depth.saturating_add(1);
    for (position, (name, value)) in fields.iter().enumerate() {
        if position != 0 {
            out.push_str(",\n");
        }
        write_indent(out, inner);
        write_text(out, name);
        out.push_str(": ");
        write_value(out, value, inner);
    }
    out.push('\n');
    write_indent(out, depth);
    out.push('}');
}

fn write_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_text(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(character),
            other => write_unicode_escape(out, other),
        }
    }
    out.push('"');
}

fn write_unicode_escape(out: &mut String, character: char) {
    let mut units = [0_u16; 2];
    for unit in character.encode_utf16(&mut units) {
        out.push_str("\\u");
        for byte in unit.to_be_bytes() {
            out.extend(char::from_digit(u32::from(byte.div_euclid(16)), 16));
            out.extend(char::from_digit(u32::from(byte.rem_euclid(16)), 16));
        }
    }
}

// json.loads reads a number with a fraction or an exponent as a float and any other as an int
fn python_number(number: &Number) -> String {
    let text = number.to_string();
    if !text.contains(['.', 'e', 'E']) {
        return if text == "-0" { "0".to_owned() } else { text };
    }
    let Ok(value) = text.parse::<f64>() else {
        return text;
    };
    let shortest = format!("{value:?}");
    match (shortest.as_str(), shortest.split_once('e')) {
        ("inf", _) => "Infinity".to_owned(),
        ("-inf", _) => "-Infinity".to_owned(),
        (plain, None) => plain.to_owned(),
        (_, Some((mantissa, power))) => power.strip_prefix('-').map_or_else(
            || format!("{mantissa}e+{power:0>2}"),
            |digits| format!("{mantissa}e-{digits:0>2}"),
        ),
    }
}
