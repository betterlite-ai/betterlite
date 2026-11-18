use super::exception::LabelException;

#[must_use]
pub fn extract_status_code(
    enum_status_code: Option<&str>,
    exception: Option<&LabelException>,
    kwargs_exception: Option<&LabelException>,
) -> Option<i64> {
    if let Some(status_code) = enum_status_code.and_then(|code| code.parse::<i64>().ok()) {
        return Some(status_code);
    }
    if let Some(status_code) = exception.and_then(exception_status_code) {
        return Some(status_code);
    }
    kwargs_exception.and_then(exception_status_code)
}

// A status_code that is present wins even when unparsable, so it never falls through to code.
fn exception_status_code(exception: &LabelException) -> Option<i64> {
    let chosen = exception
        .status_code
        .as_deref()
        .filter(|code| !code.is_empty())
        .or_else(|| exception.code.as_deref().filter(|code| !code.is_empty()))?;
    chosen.parse::<i64>().ok()
}
