use betterlite_models::decisions::Json;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Plain,
    MidStreamFallback,
}

// A Python exception as the failure state reads it: its text, detail attributes and wrapped cause.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct FailureCause {
    pub kind: FailureKind,
    pub text: String,
    #[serde(default)]
    pub message: Option<Json>,
    #[serde(default, rename = "type")]
    pub error_type: Option<Json>,
    #[serde(default)]
    pub code: Option<Json>,
    #[serde(default)]
    pub status_code: Option<Json>,
    #[serde(default)]
    pub body: Option<Json>,
    #[serde(default)]
    pub original_exception: Option<Box<Self>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum FailureCode {
    Integer(i64),
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FailureDetails {
    pub(super) message: Option<String>,
    pub(super) code: Option<FailureCode>,
    pub(super) error_type: Option<String>,
    pub(super) status_code: Option<i64>,
}

// A MidStreamFallbackError is unwrapped only while it still wraps an original_exception.
pub(super) fn original_failure(mut failure: &FailureCause) -> &FailureCause {
    loop {
        match failure.kind {
            FailureKind::Plain => return failure,
            FailureKind::MidStreamFallback => {
                let Some(original) = failure.original_exception.as_deref() else {
                    return failure;
                };
                failure = original;
            }
        }
    }
}

pub(super) fn failure_details(original: &FailureCause) -> FailureDetails {
    let mapped = FailureDetails {
        message: normalized_text(original.message.as_ref()),
        code: normalized_code(original.code.as_ref()),
        error_type: normalized_text(original.error_type.as_ref()),
        status_code: match original.status_code.as_ref() {
            Some(Json::Number(number)) => number.as_i64(),
            _ => None,
        },
    };
    let Some(Json::Object(body)) = &original.body else {
        return mapped;
    };
    FailureDetails {
        message: or_fallback(normalized_text(body.get("message")), mapped.message),
        code: normalized_code(body.get("code")).or(mapped.code),
        error_type: or_fallback(normalized_text(body.get("type")), mapped.error_type),
        status_code: mapped.status_code,
    }
}

fn normalized_text(value: Option<&Json>) -> Option<String> {
    match value {
        Some(Json::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

fn normalized_code(value: Option<&Json>) -> Option<FailureCode> {
    match value {
        Some(Json::Text(text)) => Some(FailureCode::Text(text.clone())),
        Some(Json::Number(number)) => number.as_i64().map(FailureCode::Integer),
        _ => None,
    }
}

fn or_fallback(upstream: Option<String>, mapped: Option<String>) -> Option<String> {
    match upstream {
        Some(text) if !text.is_empty() => Some(text),
        _ => mapped,
    }
}

pub(super) fn committed_text(
    observed: Option<String>,
    committed: Option<String>,
) -> Option<String> {
    match observed {
        Some(text) if !text.is_empty() => Some(text),
        _ => committed,
    }
}
