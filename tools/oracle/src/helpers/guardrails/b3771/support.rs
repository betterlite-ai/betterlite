use std::time::Duration;

use serde_json::json;

use crate::case::UpstreamAnswer;
use crate::routing_mock_scripts::{MockScript, Outcome, ScriptedReply, Wait};

pub(super) fn parse_json_field<T>(
    request: &serde_json::Value,
    name: &str,
) -> Result<Option<T>, Box<crate::helpers::HelperError>>
where
    T: serde::de::DeserializeOwned,
{
    let Some(value) = request.get(name).cloned() else {
        return Ok(None);
    };
    let parsed = serde_json::from_value(value).map_err(guardrail_error)?;
    Ok(Some(parsed))
}

pub(super) fn error_failure(
    is_blocked: bool,
    blocked_class: &str,
    other_class: &str,
    message: String,
) -> Box<crate::helpers::HelperError> {
    let class = if is_blocked { blocked_class } else { other_class };
    failure_with_class(class, message)
}

pub(super) fn failure_with_class(class_name: &str, message: String) -> Box<crate::helpers::HelperError> {
    let class = class_name.to_owned();
    Box::new(crate::helpers::HelperError {
        class,
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn guardrail_error<E>(error: E) -> Box<crate::helpers::HelperError>
where
    E: std::fmt::Display,
{
    let message = error.to_string();
    failure_with_class("GuardrailError", message)
}

pub(super) fn field_or_empty_object(input: &serde_json::Value, key: &str) -> serde_json::Value {
    input.get(key).cloned().unwrap_or_else(|| json!({}))
}

pub(super) fn current_thread_runtime() -> Result<tokio::runtime::Runtime, Box<crate::helpers::HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(guardrail_error)
}

pub(super) fn mock_script_for(
    reply: &UpstreamAnswer,
) -> Result<MockScript, Box<crate::helpers::HelperError>> {
    MockScript::try_from(std::slice::from_ref(reply)).map_err(guardrail_error)
}

// "closed" drops the connection immediately; "timeout" delays past the client's timeout.
pub(super) fn disconnection_script(fail: &str) -> MockScript {
    let wait = if fail == "closed" {
        Wait::None
    } else {
        Wait::Delay { millis: 500 }
    };
    MockScript {
        deadline: Duration::from_secs(5),
        replies: vec![ScriptedReply {
            wait,
            then: Outcome::Close,
        }],
        barriers: std::collections::BTreeMap::new(),
    }
}
