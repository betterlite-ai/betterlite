use betterlite_prompts::arguments::json::{Json, Object};

pub mod error;

use self::error::SailError;

mod tests;

pub const PROVIDER: &str = "sail";

const COMPLETION_WINDOW: &str = "completion_window";
const EXTRA_BODY: &str = "extra_body";
const METADATA: &str = "metadata";
const SERVICE_TIER: &str = "service_tier";
const DROP_PARAMS_HINT: &str = "To drop it, set `litellm.drop_params=True` or for proxy: `litellm_settings: drop_params: true`";

// B039 fills this from litellm.drop_params
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SailSettings {
    pub global_drop_params: bool,
}

// standard is only a caller-declared window, and bills as balanced
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionWindow {
    Asap,
    Balanced,
    Standard,
    Flex,
}

impl CompletionWindow {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Asap => "asap",
            Self::Balanced => "balanced",
            Self::Standard => "standard",
            Self::Flex => "flex",
        }
    }

    // LiteLLM's _BILLED_TIER_FOR_WINDOW: the tier whose price columns match the window
    const fn billed_tier(self) -> Option<&'static str> {
        match self {
            Self::Asap => None,
            Self::Balanced | Self::Standard => Some("balanced"),
            Self::Flex => Some("flex"),
        }
    }
}

enum TierWindow {
    Unknown,
    NoWindow,
    Window(CompletionWindow),
}

fn window_for_tier(tier: &str) -> TierWindow {
    match tier {
        "auto" => TierWindow::NoWindow,
        "default" | "priority" => TierWindow::Window(CompletionWindow::Asap),
        "flex" => TierWindow::Window(CompletionWindow::Flex),
        "balanced" => TierWindow::Window(CompletionWindow::Balanced),
        _ => TierWindow::Unknown,
    }
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

// ast-grep-ignore: no-long-names
pub fn completion_window_for_service_tier(
    service_tier: Option<&Json>,
    model: &str,
    drop_params: bool,
    settings: &SailSettings,
) -> Result<Option<CompletionWindow>, SailError> {
    let Some(value) = service_tier else {
        return Ok(None);
    };
    let lookup = match value {
        Json::Null => return Ok(None),
        Json::Text(raw) => window_for_tier(&raw.to_lowercase()),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => TierWindow::Unknown,
    };
    match lookup {
        TierWindow::NoWindow => Ok(None),
        TierWindow::Window(window) => Ok(Some(window)),
        TierWindow::Unknown if drop_params || settings.global_drop_params => Ok(None),
        TierWindow::Unknown => Err(SailError::UnsupportedServiceTier {
            model: model.to_owned(),
            service_tier: describe(Some(value)),
        }),
    }
}

pub fn metadata_without_caller_window(
    metadata: Option<&Json>,
    field: &str,
    model: &str,
    drop_params: bool,
    settings: &SailSettings,
) -> Result<Object, SailError> {
    let Some(Json::Object(entries)) = metadata else {
        return Ok(Object::new());
    };
    if entries.contains_key(COMPLETION_WINDOW) && !drop_params && !settings.global_drop_params {
        return Err(SailError::CallerWindowInMetadata {
            model: model.to_owned(),
            field: field.to_owned(),
        });
    }
    let mut cleaned = entries.clone();
    cleaned.shift_remove(COMPLETION_WINDOW);
    Ok(cleaned)
}

pub fn extra_body_for_sail(
    extra_body: &Object,
    request_metadata: Option<&Json>,
    model: &str,
    drop_params: bool,
    settings: &SailSettings,
) -> Result<Object, SailError> {
    if extra_body.contains_key(SERVICE_TIER) && !drop_params && !settings.global_drop_params {
        return Err(SailError::ExtraBodyServiceTier {
            model: model.to_owned(),
        });
    }
    let raw_metadata = extra_body.get(METADATA);
    let mut merged = metadata_without_caller_window(
        raw_metadata,
        "extra_body.metadata",
        model,
        drop_params,
        settings,
    )?;
    if let Some(Json::Object(request)) = request_metadata {
        merged.extend(request.clone());
    }
    let mut rest = extra_body.clone();
    rest.shift_remove(SERVICE_TIER);
    rest.shift_remove(METADATA);
    if !merged.is_empty() {
        rest.insert(String::from(METADATA), Json::Object(merged));
        return Ok(rest);
    }
    let Some(raw) = raw_metadata else {
        return Ok(rest);
    };
    if !matches!(raw, Json::Object(_)) {
        rest.insert(String::from(METADATA), raw.clone());
    }
    Ok(rest)
}

fn chat_metadata(
    request: &Object,
    model: &str,
    drop_params: bool,
    settings: &SailSettings,
) -> Result<Object, SailError> {
    // ast-grep-ignore: no-long-names
    let window = completion_window_for_service_tier(
        request.get(SERVICE_TIER),
        model,
        drop_params,
        settings,
    )?;
    let mut metadata = metadata_without_caller_window(
        request.get(METADATA),
        METADATA,
        model,
        drop_params,
        settings,
    )?;
    if let Some(window) = window {
        metadata.insert(
            String::from(COMPLETION_WINDOW),
            Json::Text(String::from(window.as_str())),
        );
    }
    Ok(metadata)
}

pub fn chat_request_for_sail(
    request: &Object,
    model: &str,
    drop_params: bool,
    settings: &SailSettings,
) -> Result<Object, SailError> {
    let metadata = chat_metadata(request, model, drop_params, settings)?;
    let mut result = request.clone();
    result.shift_remove(SERVICE_TIER);
    result.shift_remove(METADATA);
    result.shift_remove(EXTRA_BODY);
    let metadata_is_sent = !metadata.is_empty();
    if metadata_is_sent {
        result.insert(String::from(METADATA), Json::Object(metadata.clone()));
    }
    if let Some(Json::Object(extra_body)) = request.get(EXTRA_BODY) {
        let merged = extra_body_for_sail(
            extra_body,
            Some(&Json::Object(metadata)),
            model,
            drop_params,
            settings,
        )?;
        result.insert(String::from(EXTRA_BODY), Json::Object(merged));
    }
    Ok(result)
}

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
    // ast-grep-ignore: no-long-names
    let tier_window = completion_window_for_service_tier(raw_tier, model, drop_params, settings)?;
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
    if let Some(window) = window {
        wire_metadata.insert(
            String::from(COMPLETION_WINDOW),
            Json::Text(String::from(window.as_str())),
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
    caller: Option<CompletionWindow>,
    tier: Option<CompletionWindow>,
    raw_tier: Option<&Json>,
    model: &str,
) -> Result<Option<CompletionWindow>, SailError> {
    if let (Some(caller), Some(tier)) = (caller, tier)
        && caller.billed_tier() != tier.billed_tier()
    {
        return Err(SailError::ConflictingWindows {
            model: model.to_owned(),
            service_tier: describe(raw_tier),
            completion_window: format!("{:?}", caller.as_str()),
        });
    }
    Ok(caller.or(tier))
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

// Python's repr of an arbitrary value is not reproducible: a string keeps Rust's debug text and anything else its JSON text, see docs/deviations/B6411.yaml
fn describe(value: Option<&Json>) -> String {
    match value {
        Some(Json::Text(text)) => format!("{text:?}"),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
        None => String::from("null"),
    }
}
