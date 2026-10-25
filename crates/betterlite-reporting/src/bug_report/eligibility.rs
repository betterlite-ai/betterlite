pub const DISABLE_ENV_VAR: &str = "LITELLM_DISABLE_BUG_REPORT_LINK";

#[must_use]
pub fn bug_report_enabled(disable_link: Option<&str>) -> bool {
    let disabled = disable_link.is_some_and(|value| value.to_lowercase() == "true");
    !disabled
}

// An error that carries an HTTP status already reached the caller as a handled failure
#[must_use]
pub fn should_report_bug(disable_link: Option<&str>, status_code: Option<u16>) -> bool {
    let enabled = bug_report_enabled(disable_link);
    enabled && status_code.is_none()
}
