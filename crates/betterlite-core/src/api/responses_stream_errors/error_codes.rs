use super::failure::{FailureCode, FailureDetails};
use unicode_general_category::{GeneralCategory, get_general_category};

pub(super) fn response_error_code(details: &FailureDetails) -> String {
    if let Some(code) = special_code(details.code.as_ref()) {
        return code;
    }
    if let Some(code) = details.error_type.as_deref().and_then(special_text_code) {
        return code;
    }
    if let Some(FailureCode::Text(code)) = &details.code
        && !code.is_empty()
        && !is_decimal(code)
    {
        return code.clone();
    }
    status_error_code(details.status_code)
}

fn special_code(value: Option<&FailureCode>) -> Option<String> {
    match value {
        Some(FailureCode::Integer(429)) => Some("rate_limit_exceeded".to_owned()),
        Some(FailureCode::Text(text)) => special_text_code(text),
        _ => None,
    }
}

fn special_text_code(text: &str) -> Option<String> {
    if text == "insufficient_quota" {
        return Some("insufficient_quota".to_owned());
    }
    if text == "429" || text.starts_with("rate_limit") {
        return Some("rate_limit_exceeded".to_owned());
    }
    None
}

pub(super) fn status_error_code(status_code: Option<i64>) -> String {
    let Some(value) = status_code else {
        return "server_error".to_owned();
    };
    if !(400..500).contains(&value) {
        return "server_error".to_owned();
    }
    match value {
        401 => "authentication_error".to_owned(),
        403 => "permission_error".to_owned(),
        404 => "not_found_error".to_owned(),
        408 => "request_timeout".to_owned(),
        429 => "rate_limit_exceeded".to_owned(),
        _ => "invalid_request_error".to_owned(),
    }
}

// CPython's str.isdecimal(): non-empty, every character in category Decimal_Number.
pub(super) fn is_decimal(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|character| get_general_category(character) == GeneralCategory::DecimalNumber)
}
