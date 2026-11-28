use std::collections::BTreeMap;

use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};
use betterlite_openai::api::responses_shapes::{GenericResponseOutputItem, OutputText};
use indexmap::IndexMap;

use crate::api::responses_mcp_results::McpToolResult;

use super::SHORT_ID_LENGTH;
use super::records::IdSource;

const TOOLS_FETCHED_TYPE: &str = "mcp_tools_fetched";
const TOOLS_FETCHED_PREFIX: &str = "mcp_tools_";
const TOOL_RESULTS_TYPE: &str = "tool_execution_results";
const TOOL_RESULTS_PREFIX: &str = "tool_results_";
const COMPLETED_STATUS: &str = "completed";
const SYSTEM_ROLE: &str = "system";
const OUTPUT_TEXT_TYPE: &str = "output_text";

// _add_mcp_output_elements_to_response (L1310-L1357): the supplied response grows by fetched tools, then tool results
pub fn append_mcp_output_items(
    response: &mut ResponsesApiResponse,
    mcp_tools_fetched: &[Json],
    tool_results: &[McpToolResult],
    ids: &IdSource,
) {
    let fetched_text = pretty_text(&Json::List(mcp_tools_fetched.to_vec()));
    let results_text = pretty_text(&tool_results_value(tool_results));
    let fetched = output_item(TOOLS_FETCHED_TYPE, TOOLS_FETCHED_PREFIX, &fetched_text, ids);
    let results = output_item(TOOL_RESULTS_TYPE, TOOL_RESULTS_PREFIX, &results_text, ids);
    response.output.push(ResponseOutputItem::Message(fetched));
    response.output.push(ResponseOutputItem::Message(results));
}

fn output_item(kind: &str, prefix: &str, text: &str, ids: &IdSource) -> GenericResponseOutputItem {
    let suffix: String = ids().chars().take(SHORT_ID_LENGTH).collect();
    GenericResponseOutputItem {
        r#type: kind.to_owned(),
        id: format!("{prefix}{suffix}"),
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

// the producer's ToolResult fields as json.dumps receives them; betterlite's JSON maps sort keys, so name, result, tool_call_id
fn tool_results_value(tool_results: &[McpToolResult]) -> Json {
    Json::List(
        tool_results
            .iter()
            .map(|result| {
                Json::Object(IndexMap::from([
                    (
                        String::from("name"),
                        result
                            .name
                            .as_ref()
                            .map_or(Json::Null, |name| Json::Text(name.clone())),
                    ),
                    (String::from("result"), Json::Text(result.result.clone())),
                    (
                        String::from("tool_call_id"),
                        result
                            .tool_call_id
                            .as_ref()
                            .map_or(Json::Null, |id| Json::Text(id.clone())),
                    ),
                ]))
            })
            .collect(),
    )
}

// json.dumps(value, indent=2, default=str): Python's layout, with python_json's escaping and number text for a scalar
fn pretty_text(value: &Json) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0);
    out
}

fn write_value(out: &mut String, value: &Json, depth: usize) {
    match value {
        Json::List(items) => write_list(out, items, depth),
        Json::Object(fields) => write_object(out, fields, depth),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {
            out.push_str(&python_json::dumps(value));
        }
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
        out.push_str(&python_json::dumps(&Json::Text(name.clone())));
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
