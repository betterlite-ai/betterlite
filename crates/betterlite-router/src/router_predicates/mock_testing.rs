use std::collections::BTreeMap;

use betterlite_error::error::details::{RateLimitDetails, ResponseDetails};
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_models::decisions::Json;

use super::JsonMap;
use super::map_values::object_value;
use super::source_text::source_text;

// the space before LiteLLM's line-continuation backslash plus the continuation line's 20-space indent
const JOINED_INDENT: &str = "                     ";

#[derive(Debug, Eq, PartialEq)]
pub struct MockTestingParams {
    pub fallbacks: Option<bool>,
    pub context_window: Option<bool>,
    pub content_policy: Option<bool>,
}

impl MockTestingParams {
    #[must_use]
    pub fn take_from(request: &mut JsonMap) -> Self {
        Self {
            fallbacks: request
                .shift_remove("mock_testing_fallbacks")
                .as_ref()
                .and_then(switch_value),
            context_window: request
                .shift_remove("mock_testing_context_fallbacks")
                .as_ref()
                .and_then(switch_value),
            content_policy: request
                .shift_remove("mock_testing_content_policy_fallbacks")
                .as_ref()
                .and_then(switch_value),
        }
    }
}

pub fn raise_mock_fallback_error(
    request: &mut JsonMap,
    model_group: Option<&str>,
    fallbacks: Option<&Json>,
    context_window_fallbacks: Option<&Json>,
    content_policy_fallbacks: Option<&Json>,
) -> Result<(), ErrorClass> {
    let switches = MockTestingParams::take_from(request);
    let group = model_group.unwrap_or("None");
    let error = if switches.fallbacks == Some(true) {
        let message = format!(
            "This is a mock exception for model={group}, to trigger a fallback. Fallbacks={}",
            message_value(fallbacks)
        );
        ErrorClass::InternalServer(Box::new(mock_context(message, model_group, None)))
    } else if switches.context_window == Some(true) {
        let message = format!(
            "This is a mock exception for model={group}, to trigger a fallback.{JOINED_INDENT}Context_Window_Fallbacks={}",
            message_value(context_window_fallbacks)
        );
        ErrorClass::ContextWindowExceeded(Box::new(ResponseDetails::from(mock_context(
            message,
            model_group,
            None,
        ))))
    } else if switches.content_policy == Some(true) {
        let message = format!(
            "This is a mock exception for model={group}, to trigger a fallback.{JOINED_INDENT}Context_Policy_Fallbacks={}",
            message_value(content_policy_fallbacks)
        );
        ErrorClass::ContentPolicyViolation(Box::new(ResponseDetails::from(mock_context(
            message,
            model_group,
            None,
        ))))
    } else {
        return Ok(());
    };
    Err(error)
}

pub fn raise_mock_rate_limit(
    request: &mut JsonMap,
    model_group: Option<&str>,
    group_deployments: &[JsonMap],
) -> Result<(), ErrorClass> {
    let mock_error = request.shift_remove("mock_testing_rate_limit_error");
    let num_retries = match group_deployments {
        [deployment] => deployment
            .get("litellm_params")
            .and_then(object_value)
            .and_then(|params| params.get("num_retries"))
            .and_then(retry_count),
        _ => None,
    };
    if !matches!(mock_error, Some(Json::Bool(true))) {
        return Ok(());
    }
    let message = format!(
        "This is a mock exception for model={}, to trigger a rate limit error.",
        model_group.unwrap_or("None")
    );
    Err(ErrorClass::RateLimit(Box::new(RateLimitDetails {
        context: mock_context(message, model_group, num_retries),
        category: RateLimitCategory::LitellmRateLimit,
        dimension: None,
        headers: BTreeMap::new(),
    })))
}

fn switch_value(value: &Json) -> Option<bool> {
    match value {
        Json::Bool(flag) => Some(*flag),
        Json::Text(text) => match text.trim().to_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        Json::Null | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn retry_count(value: &Json) -> Option<u32> {
    let Json::Number(number) = value else {
        return None;
    };
    let count = number.as_u64()?;
    u32::try_from(count).ok()
}

fn message_value(value: Option<&Json>) -> String {
    match value {
        None | Some(Json::Null) => String::from("None"),
        Some(found) => source_text(found),
    }
}

fn mock_context(
    message: String,
    model_group: Option<&str>,
    num_retries: Option<u32>,
) -> ErrorContext {
    ErrorContext {
        message,
        model: model_group.map(str::to_owned),
        provider: Some(String::new()),
        max_retries: None,
        num_retries,
        body_type: None,
        body_param: None,
        body_code: None,
    }
}
