use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
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
    let serialized = safe_dumps(modified, DEFAULT_MAX_RECURSE_DEPTH, None).ok()?;
    let suffix = if carriage_return { "\r" } else { "" };
    Some(format!("data: {serialized}{suffix}"))
}
