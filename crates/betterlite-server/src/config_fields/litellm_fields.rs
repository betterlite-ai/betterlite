use betterlite_config::sections::settings::JsonValue;
use serde_json::Number;

use super::error::ConfigFieldError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiFieldType {
    Float,
    Dollar,
    Boolean,
    Select,
}

impl UiFieldType {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Float => "Float",
            Self::Dollar => "Dollar",
            Self::Boolean => "Boolean",
            Self::Select => "Select",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiDefault {
    Null,
    False,
    One,
}

// one _GENERAL_SETTINGS_UI_LITELLM_FIELDS entry: a litellm module attribute the admin UI may edit
#[derive(Clone, Copy, Debug)]
pub struct UiLitellmField {
    pub name: &'static str,
    pub kind: UiFieldType,
    pub options: Option<&'static [&'static str]>,
    pub tab: Option<&'static str>,
    pub default: UiDefault,
    pub description: &'static str,
}

const TAB: Option<&str> = Some("prompt_caching");
const TTL_OPTIONS: &[&str] = &["5m", "1h"];

// _GENERAL_SETTINGS_UI_LITELLM_FIELDS order, which is the order /config/list appends them in
pub static UI_FIELDS: [UiLitellmField; 6] = [
    UiLitellmField {
        name: "budget_exceeded_throttle_percentage",
        kind: UiFieldType::Float,
        options: None,
        tab: None,
        default: UiDefault::Null,
        description: "Fraction (0, 1] of a key's configured TPM/RPM that an over-budget key with 'Throttle on budget exceeded' enabled keeps serving at. Leave empty to hard-block over-budget keys.",
    },
    UiLitellmField {
        name: "enable_anthropic_prompt_caching",
        kind: UiFieldType::Boolean,
        options: None,
        tab: TAB,
        default: UiDefault::False,
        description: "Auto-adds cache_control to the system prompt and trailing turn for supported Claude models on Anthropic, Bedrock, Vertex AI, and Azure AI. The cache is shared across callers on the same upstream credentials.",
    },
    UiLitellmField {
        name: "anthropic_prompt_caching_ttl",
        kind: UiFieldType::Select,
        options: Some(TTL_OPTIONS),
        tab: TAB,
        default: UiDefault::Null,
        description: "Empty uses Anthropic's 5m default. 1h suits long sessions but doubles the cache write cost.",
    },
    UiLitellmField {
        name: "openai_system_messages_first",
        kind: UiFieldType::Boolean,
        options: None,
        tab: TAB,
        default: UiDefault::False,
        description: "Moves system and developer messages to the front of the messages array on OpenAI and Azure OpenAI chat completions requests, keeping their relative order. OpenAI's prompt cache matches on the exact prefix, so a system message that arrives mid-conversation otherwise breaks the cached prefix on every turn.",
    },
    UiLitellmField {
        name: "budget_rollover",
        kind: UiFieldType::Boolean,
        options: None,
        tab: None,
        default: UiDefault::False,
        description: "Carry spend beyond max_budget into the next window when budgets reset, instead of forgiving it. Applies to key, user, team, team member, org, tag and end-user budgets.",
    },
    UiLitellmField {
        name: "max_ui_session_budget",
        kind: UiFieldType::Dollar,
        options: None,
        tab: None,
        default: UiDefault::One,
        description: "USD spend cap for each dashboard login session; covers LLM calls made from the dashboard such as the playground and auto Router Test Connection. Each login starts a fresh session with this budget. Clearing restores the $1 default.",
    },
];

// a cleared field falls back to the spec default, then false for a Boolean, then None
#[must_use]
pub fn default_value(field: UiLitellmField) -> JsonValue {
    match field.default {
        UiDefault::Null => JsonValue::Null,
        UiDefault::False => JsonValue::Bool(false),
        UiDefault::One => Number::from_f64(1.0).map_or(JsonValue::Null, JsonValue::Number),
    }
}

// _validate_general_settings_ui_litellm_value: null or "" gives the default, anything else its type rule
pub fn validate_ui_value(
    field: UiLitellmField,
    value: &JsonValue,
) -> Result<JsonValue, ConfigFieldError> {
    if value.is_null() || value.as_str() == Some("") {
        return Ok(default_value(field));
    }
    match field.kind {
        UiFieldType::Boolean => boolean_value(field, value),
        UiFieldType::Select => select_value(field, value),
        UiFieldType::Float => numeric_value(field, value, false),
        UiFieldType::Dollar => numeric_value(field, value, true),
    }
}

fn boolean_value(field: UiLitellmField, value: &JsonValue) -> Result<JsonValue, ConfigFieldError> {
    let JsonValue::Bool(flag) = value else {
        return Err(ui_bad_request(field, "must be true or false"));
    };
    Ok(JsonValue::Bool(*flag))
}

fn select_value(field: UiLitellmField, value: &JsonValue) -> Result<JsonValue, ConfigFieldError> {
    let options = field.options.unwrap_or_default();
    let message = format!("must be one of: {}, or empty", options.join(", "));
    let JsonValue::String(text) = value else {
        return Err(ui_bad_request(field, &message));
    };
    if !options.contains(&text.as_str()) {
        return Err(ui_bad_request(field, &message));
    }
    Ok(JsonValue::String(text.clone()))
}

// <field name> <rule>, the sentences _validate_general_settings_ui_litellm_value raises
fn ui_bad_request(field: UiLitellmField, message: &str) -> ConfigFieldError {
    ConfigFieldError::BadRequest {
        detail: format!("{} {message}", field.name),
    }
}

// booleans are numbers in Python, so the number rules only see a real JSON number
fn numeric_value(
    field: UiLitellmField,
    value: &JsonValue,
    dollar: bool,
) -> Result<JsonValue, ConfigFieldError> {
    let message = if dollar {
        format!("{} must be a positive dollar amount or empty", field.name)
    } else {
        format!("{} must be a number in (0, 1] or empty", field.name)
    };
    let Some(numeric) = value.as_f64() else {
        return Err(ConfigFieldError::BadRequest { detail: message });
    };
    let allowed = if dollar {
        numeric > 0.0
    } else {
        numeric > 0.0 && numeric <= 1.0
    };
    if !allowed {
        return Err(ConfigFieldError::BadRequest { detail: message });
    }
    Number::from_f64(numeric)
        .map(JsonValue::Number)
        .ok_or(ConfigFieldError::BadRequest { detail: message })
}
