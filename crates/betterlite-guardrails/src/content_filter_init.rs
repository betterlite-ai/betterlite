use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;

pub mod args;
pub mod error;
mod tests;

use args::ContentFilterArgs;
use error::ContentFilterInitError;

const IMAGE_MODEL: &str = "image_model";
const COMPETITOR_INTENT_CONFIG: &str = "competitor_intent_config";
// LiteLLM's getattr fallback, which its class then matches against as the medium threshold
const DEFAULT_SEVERITY: &str = "medium";

pub fn initialize_content_filter(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
    guardrail_id: Option<&str>,
    policy_template: Option<&str>,
    router_present: bool,
) -> Result<ContentFilterArgs, ContentFilterInitError> {
    let Some(name) = guardrail_name.filter(|name| !name.is_empty()) else {
        return Err(ContentFilterInitError::MissingName);
    };
    Ok(ContentFilterArgs {
        name: name.to_owned(),
        guardrail_id: guardrail_id.map(str::to_owned),
        policy_template: policy_template.map(str::to_owned),
        patterns: params
            .base
            .content_filter
            .patterns
            .clone()
            .map(|checked| checked.0),
        blocked_words: params.base.content_filter.blocked_words.clone(),
        blocked_words_file: params.base.content_filter.blocked_words_file.clone(),
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
        categories: params.base.content_filter.categories.clone(),
        severity_threshold: params
            .base
            .content_filter
            .severity_threshold
            .clone()
            .unwrap_or_else(|| DEFAULT_SEVERITY.to_owned()),
        router_present,
        image_model: extra_text(&params.base.extra, IMAGE_MODEL)?,
        competitor_intent_config: extra_object(&params.base.extra, COMPETITOR_INTENT_CONFIG)?,
        end_session_after_n_fails: params.base.fields.end_session_after_n_fails,
        on_violation: params.base.fields.on_violation,
        realtime_violation_message: params.base.fields.realtime_violation_message.clone(),
    })
}

fn extra_text(extra: &OpaqueParams, key: &str) -> Result<Option<String>, ContentFilterInitError> {
    match extra.0.get(key) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(text.clone())),
        Some(_wrong) => Err(ContentFilterInitError::InvalidParam(key.to_owned())),
    }
}

fn extra_object(
    extra: &OpaqueParams,
    key: &str,
) -> Result<Option<OpaqueParams>, ContentFilterInitError> {
    match extra.0.get(key) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Object(fields)) => Ok(Some(OpaqueParams(fields.clone()))),
        Some(_wrong) => Err(ContentFilterInitError::InvalidParam(key.to_owned())),
    }
}
