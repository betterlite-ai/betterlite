use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, MAX_DEPTH_MARKER};
use betterlite_models::decisions::Json;

use super::StreamCostInjector;
use super::projection;

const DATA_PREFIX: &str = "data:";

// LiteLLM rewrites only the first data line whose usage it can price and returns at once.
pub(super) fn inject_into_frame(
    frame: &str,
    model_name: &str,
    injector: StreamCostInjector<'_>,
    mut log: Option<&mut LoggingState>,
) -> Option<String> {
    let mut lines: Vec<&str> = frame.split('\n').collect();
    let mut found: Option<(usize, String)> = None;
    for (position, line) in lines.iter().enumerate() {
        let Some(payload) = payload_of(line) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Json>(payload) else {
            return None;
        };
        if !matches!(parsed, Json::Object(_)) {
            return None;
        }
        let state = log.as_deref_mut();
        match projection::inject_into_object(&parsed, model_name, injector, state) {
            Ok(Some(modified)) => {
                found = Some((position, data_line(&modified, line.ends_with('\r'))?));
                break;
            }
            Ok(None) => {}
            Err(_) => return None,
        }
    }
    let (position, replacement) = found?;
    let slot = lines.get_mut(position)?;
    *slot = replacement.as_str();
    Some(lines.join("\n"))
}

fn payload_of(line: &str) -> Option<&str> {
    let stripped = line.trim();
    let value = stripped.strip_prefix(DATA_PREFIX)?;
    let payload = value.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    Some(payload)
}

fn data_line(modified: &Json, carriage_return: bool) -> Option<String> {
    let serialized = serde_json::to_string(&bounded_value(modified, 0)).ok()?;
    let suffix = if carriage_return { "\r" } else { "" };
    Some(format!("data: {serialized}{suffix}"))
}

// B192's safe_dumps sorts keys, so the parsed value gets the same depth marker and NUL stripping.
fn bounded_value(value: &Json, depth: usize) -> Json {
    if depth > DEFAULT_MAX_RECURSE_DEPTH {
        return Json::Text(MAX_DEPTH_MARKER.to_owned());
    }
    match value {
        Json::Null | Json::Bool(_) | Json::Number(_) => value.clone(),
        Json::Text(text) => Json::Text(text.replace('\0', "")),
        Json::List(items) => Json::List(
            items
                .iter()
                .map(|item| bounded_value(item, depth.saturating_add(1)))
                .collect(),
        ),
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, item)| {
                    let clean_name = name.replace('\0', "");
                    let child = bounded_value(item, depth.saturating_add(1));
                    (clean_name, child)
                })
                .collect(),
        ),
    }
}
