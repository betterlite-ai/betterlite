use betterlite_prompts::arguments::json::{Json, Object};

use super::error::SailError;
// ast-grep-ignore: no-long-names
use super::completion_window_for_service_tier as window_for_tier;
use super::{COMPLETION_WINDOW, CompletionWindow, METADATA, SERVICE_TIER, SailSettings, describe};

// ast-grep-ignore: no-long-names
pub fn responses_params_with_completion_window(
    params: &Object,
    model: &str,
    drop_params: bool,
    settings: &SailSettings,
) -> Result<Object, SailError> {
    let raw_tier = params.get(SERVICE_TIER);
    let raw_metadata = params.get(METADATA);
    let metadata = raw_metadata.and_then(Json::as_object);
    let tier_window = window_for_tier(raw_tier, model, drop_params, settings)?;
    let caller_window = match metadata.and_then(|entries| entries.get(COMPLETION_WINDOW)) {
        Some(window) => caller_completion_window(window, model, drop_params, settings)?,
        None => None,
    };
    let window = resolved_window(caller_window, tier_window, raw_tier, model)?;
    let mut result = params.clone();
    result.shift_remove(SERVICE_TIER);
    result.shift_remove(METADATA);
    let mut wire_metadata = metadata.cloned().unwrap_or_default();
    wire_metadata.shift_remove(COMPLETION_WINDOW);
    if let Some(selected) = window {
        wire_metadata.insert(
            String::from(COMPLETION_WINDOW),
            Json::Text(String::from(selected.as_str())),
        );
    }
    let raw_is_set = raw_metadata.is_some_and(|value| !matches!(value, Json::Null));
    if !wire_metadata.is_empty() || raw_is_set {
        result.insert(String::from(METADATA), Json::Object(wire_metadata));
    }
    if let Some(billed) = window.and_then(CompletionWindow::billed_tier) {
        result.insert(String::from(SERVICE_TIER), Json::Text(String::from(billed)));
    }
    Ok(result)
}

fn resolved_window(
    caller_window: Option<CompletionWindow>,
    tier_window: Option<CompletionWindow>,
    raw_tier: Option<&Json>,
    model: &str,
) -> Result<Option<CompletionWindow>, SailError> {
    if let (Some(caller), Some(tier)) = (caller_window, tier_window)
        && caller.billed_tier() != tier.billed_tier()
    {
        return Err(SailError::ConflictingWindows {
            model: model.to_owned(),
            service_tier: describe(raw_tier),
            completion_window: format!("{:?}", caller.as_str()),
        });
    }
    Ok(caller_window.or(tier_window))
}

fn caller_completion_window(
    window: &Json,
    model: &str,
    drop_params: bool,
    settings: &SailSettings,
) -> Result<Option<CompletionWindow>, SailError> {
    if let Json::Text(text) = window
        && let Some(parsed) = caller_window_for_text(&text.to_lowercase())
    {
        return Ok(Some(parsed));
    }
    if drop_params || settings.global_drop_params {
        return Ok(None);
    }
    Err(SailError::UnsupportedCallerWindow {
        model: model.to_owned(),
        completion_window: describe(Some(window)),
    })
}

fn caller_window_for_text(text: &str) -> Option<CompletionWindow> {
    match text {
        "asap" => Some(CompletionWindow::Asap),
        "balanced" => Some(CompletionWindow::Balanced),
        "standard" => Some(CompletionWindow::Standard),
        "flex" => Some(CompletionWindow::Flex),
        _ => None,
    }
}
