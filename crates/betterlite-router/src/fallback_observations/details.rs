use betterlite_error::error::ErrorClass;
use betterlite_redact::secret::Redactor;

use super::error_info::family_fields;

// litellm.constants.ROUTER_FALLBACK_ERROR_DETAIL_MAX_CHARS, which no environment override reaches
pub const FALLBACK_DETAIL_MAX_CHARS: usize = 2000;

// truncate_fallback_error_detail: Python slices code points, so a cut never splits a character in the kept prefix
#[must_use]
pub fn truncate_fallback_error_detail(detail: &str) -> String {
    let length = detail.chars().count();
    if length <= FALLBACK_DETAIL_MAX_CHARS {
        return detail.to_owned();
    }
    let kept: String = detail.chars().take(FALLBACK_DETAIL_MAX_CHARS).collect();
    let dropped = length.saturating_sub(FALLBACK_DETAIL_MAX_CHARS);
    format!("{kept}... [truncated {dropped} characters]")
}

// Router.async_function_with_fallbacks redacts the exception text before it bounds it, so no secret survives partway into the prefix
#[must_use]
pub fn fallback_error_detail(redactor: &Redactor, class: &ErrorClass) -> String {
    let message = family_fields(class).message;
    let redacted = redactor.redact_string(&message);
    truncate_fallback_error_detail(&redacted)
}
