use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use super::JsonMap;
use super::map_values::{declared_text, text_value};

#[must_use]
pub fn as_output_cap(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => cap_from_float(number.as_f64()?),
        Json::Text(text) => separated_float(text.trim()).and_then(cap_from_float),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// Python's float() takes an underscore only between two digits
fn separated_float(text: &str) -> Option<f64> {
    let mut cleaned = String::with_capacity(text.len());
    let mut previous: Option<char> = None;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '_' {
            let between_digits = previous.is_some_and(|before| before.is_ascii_digit())
                && characters.peek().is_some_and(char::is_ascii_digit);
            if !between_digits {
                return None;
            }
        } else {
            cleaned.push(character);
        }
        previous = Some(character);
    }
    cleaned.parse().ok()
}

fn cap_from_float(value: f64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    let truncated = value.trunc();
    if truncated < 0.0_f64 {
        return None;
    }
    // Decimal rejects only finite values past its own range, which is far past i64's range too
    let cap = Decimal::from_f64(truncated).unwrap_or(Decimal::MAX);
    // int(float(value)) keeps every finite value, so an oversized cap saturates instead of disappearing
    Some(i64::try_from(cap).unwrap_or(i64::MAX))
}

#[must_use]
pub fn declared_param_allowlist(params: &JsonMap) -> BTreeSet<String> {
    let Some(Json::List(entries)) = params.get("allowed_openai_params") else {
        return BTreeSet::new();
    };
    entries
        .iter()
        .filter_map(text_value)
        .map(str::to_owned)
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedProvider {
    pub model: String,
    pub custom_llm_provider: String,
}

pub trait ParamSupport {
    fn is_authenticating_provider(&self, model: &str, custom_llm_provider: Option<&str>) -> bool;

    // None when the provider cannot be resolved; LiteLLM's broad except keeps the param then
    fn resolve_provider(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<ResolvedProvider>;

    // None also covers a provider that declares no supported list, which keeps the param too
    fn supported_openai_params(
        &self,
        model: &str,
        custom_llm_provider: &str,
        base_model: Option<&str>,
    ) -> Option<Vec<String>>;
}

#[must_use]
pub fn deployment_accepts_param<S>(
    deployment: &JsonMap,
    group: &str,
    param: &str,
    support: &S,
) -> bool
where
    S: ParamSupport,
{
    let Some(Json::Object(params)) = deployment.get("litellm_params") else {
        return true;
    };
    if params.is_empty() {
        return true;
    }
    if declared_param_allowlist(params).contains(param) {
        return true;
    }
    let custom_llm_provider = params.get("custom_llm_provider").and_then(text_value);
    if support.is_authenticating_provider(declared_model(params).unwrap_or(""), custom_llm_provider)
    {
        return true;
    }
    let Some(resolved) =
        support.resolve_provider(declared_model(params).unwrap_or(group), custom_llm_provider)
    else {
        return true;
    };
    support
        .supported_openai_params(
            &resolved.model,
            &resolved.custom_llm_provider,
            base_model(deployment, params),
        )
        .is_none_or(|supported| supported.iter().any(|name| name == param))
}

fn declared_model(params: &JsonMap) -> Option<&str> {
    match params.get("model") {
        Some(Json::Text(text)) if !text.is_empty() => Some(text),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    }
}

fn base_model<'deployment>(
    deployment: &'deployment JsonMap,
    params: &'deployment JsonMap,
) -> Option<&'deployment str> {
    let from_info = match deployment.get("model_info") {
        Some(Json::Object(info)) => declared_text(info.get("base_model")),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    };
    from_info.or_else(|| declared_text(params.get("base_model")))
}
