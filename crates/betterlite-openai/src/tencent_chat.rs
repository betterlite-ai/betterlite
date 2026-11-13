use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::openai_base_config::{CatalogCapabilities, Object, OpenAiConfig};

mod tests;

pub const DEFAULT_API_BASE: &str = "https://tokenhub-intl.tencentcloudmaas.com/v1";
pub const PROVIDER: &str = "tencent";

// LiteLLM chains the explicit base and key, then TENCENT_API_BASE and TENCENT_API_KEY, then the default base
#[must_use]
pub fn provider_info(
    api_base: Option<&str>,
    env_base: Option<&str>,
    api_key: Option<&SecretString>,
    env_key: Option<&SecretString>,
) -> (String, Option<SecretString>) {
    let base = api_base
        .filter(|value| !value.is_empty())
        .or(env_base.filter(|value| !value.is_empty()))
        .unwrap_or(DEFAULT_API_BASE)
        .to_owned();
    let key = api_key
        .filter(|key| !key.expose_secret().is_empty())
        .or(env_key.filter(|key| !key.expose_secret().is_empty()))
        .cloned();
    (base, key)
}

// the base loses every trailing slash before the suffix checks, unlike MiniMax and ModelScope
#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    let base = api_base
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_API_BASE);
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        return trimmed.to_owned();
    }
    let with_version = if trimmed.ends_with("/v1") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/v1")
    };
    format!("{with_version}/chat/completions")
}

// LiteLLM appends both reasoning names only when the capability lookup says the model reasons
#[must_use]
pub fn supported_params(
    model: &str,
    config: &OpenAiConfig,
    catalog: &CatalogCapabilities,
    supports_reasoning: bool,
) -> Vec<&'static str> {
    let mut params = config.supported_params(model, catalog);
    if supports_reasoning {
        params.push("thinking");
        params.push("reasoning_effort");
    }
    params
}

pub struct TencentParams<'config> {
    pub config: &'config OpenAiConfig,
    pub catalog: &'config CatalogCapabilities,
    pub supports_reasoning: bool,
    pub supports_adaptive_thinking: bool,
}

impl TencentParams<'_> {
    // both names leave the top-level options; only the resolved payload travels inside extra_body.thinking
    pub fn map(&self, model: &str, non_default_params: &Object, optional_params: &mut Object) {
        let supported = supported_params(model, self.config, self.catalog, self.supports_reasoning);
        for (name, value) in non_default_params {
            if supported.contains(&name.as_str()) {
                optional_params.insert(name.clone(), value.clone());
            }
        }
        let thinking = optional_params.remove("thinking");
        let effort = optional_params.remove("reasoning_effort");
        let payload = thinking_payload(
            thinking.as_ref(),
            effort.as_ref(),
            self.supports_adaptive_thinking,
        );
        if let Some(chosen) = payload {
            let mut extra_body = Object::new();
            extra_body.insert(String::from("thinking"), Value::Object(chosen));
            optional_params.insert(String::from("extra_body"), Value::Object(extra_body));
        }
    }
}

// a thinking object is used as given; otherwise a reasoning_effort string picks disabled for none and enabled for the rest
#[must_use]
pub fn thinking_payload(
    thinking: Option<&Value>,
    effort: Option<&Value>,
    supports_adaptive_thinking: bool,
) -> Option<Object> {
    if let Some(Value::Object(fields)) = thinking {
        return Some(coerce_thinking_type(fields, supports_adaptive_thinking));
    }
    let effort_text = effort.and_then(Value::as_str)?;
    let mut payload = Object::new();
    payload.insert(
        String::from("type"),
        Value::String(String::from(if effort_text == "none" {
            "disabled"
        } else {
            "enabled"
        })),
    );
    Some(coerce_thinking_type(&payload, supports_adaptive_thinking))
}

// only an enabled payload on an adaptive model changes, and only an integer budget survives the rewrite
fn coerce_thinking_type(thinking: &Object, supports_adaptive_thinking: bool) -> Object {
    if thinking.get("type").and_then(Value::as_str) != Some("enabled")
        || !supports_adaptive_thinking
    {
        return thinking.clone();
    }
    let mut coerced = Object::new();
    coerced.insert(
        String::from("type"),
        Value::String(String::from("adaptive")),
    );
    if let Some(budget) = thinking.get("budget_tokens").and_then(json_integer) {
        coerced.insert(String::from("budget_tokens"), budget.clone());
    }
    coerced
}

// LiteLLM keeps a budget only when it is a Python int, so a JSON literal with no fraction or exponent qualifies
fn json_integer(value: &Value) -> Option<&Value> {
    let Value::Number(number) = value else {
        return None;
    };
    let literal = number.to_string();
    let digits = literal.strip_prefix('-').unwrap_or(&literal);
    let integer = !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit());
    integer.then_some(value)
}
