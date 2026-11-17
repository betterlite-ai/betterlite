use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::response_content::wire::{AnthropicCompletionResponse, AnthropicToolCall, json_text};
use crate::response_content::{ResponseContentError, project_response};

const RESPONSE_FORMAT_TOOL_NAME: &str = "json_tool_call";

pub(super) struct Selection {
    pub(super) message: Option<String>,
    pub(super) tool_calls: Vec<AnthropicToolCall>,
    pub(super) extra: Option<String>,
}

// LiteLLM resolves the internal tool after the reverse map, so the second projection reads the renamed blocks
pub(super) fn select(
    body: &mut AnthropicCompletionResponse,
    json_mode: Option<bool>,
    tool_calls: &[AnthropicToolCall],
    reverse: Option<&IndexMap<String, String>>,
) -> Result<Selection, ResponseContentError> {
    let internal = internal_count(tool_calls);
    if json_mode != Some(true) {
        return Ok(unresolved(tool_calls));
    }
    if internal == 0 {
        return Ok(unresolved(tool_calls));
    }
    let replace = internal == tool_calls.len()
        && tool_calls
            .first()
            .is_some_and(|call| call.function.arguments.is_some());
    if let Some(renames) = reverse {
        reverse_content_names(body, renames);
    }
    let mut detailed = project_response(body, json_mode)?;
    let message = if replace {
        detailed.extra_content.take()
    } else {
        None
    };
    Ok(Selection {
        message,
        tool_calls: detailed.tool_calls,
        extra: detailed.extra_content,
    })
}

// LiteLLM leaves the extracted calls and text untouched when JSON mode has nothing to resolve
fn unresolved(tool_calls: &[AnthropicToolCall]) -> Selection {
    Selection {
        message: None,
        tool_calls: tool_calls.to_vec(),
        extra: None,
    }
}

fn internal_count(tool_calls: &[AnthropicToolCall]) -> usize {
    tool_calls
        .iter()
        .filter(|call| call.function.name.as_deref() == Some(RESPONSE_FORMAT_TOOL_NAME))
        .count()
}

fn reverse_content_names(
    body: &mut AnthropicCompletionResponse,
    reverse: &IndexMap<String, String>,
) {
    for block in &mut body.content {
        let Json::Object(fields) = block else {
            continue;
        };
        let Some(name) = fields.get("name").and_then(json_text) else {
            continue;
        };
        let Some(original) = reverse.get(name) else {
            continue;
        };
        fields.insert("name".to_owned(), Json::Text(original.clone()));
    }
}
