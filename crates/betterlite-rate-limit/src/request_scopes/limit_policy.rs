mod tests;

// LiteLLM's _should_enforce_rate_limit: only a dynamic type can relax, and only while the model fails
#[must_use]
pub fn should_enforce_rate_limit(limit_type: Option<&str>, model_has_failures: bool) -> bool {
    if limit_type == Some("dynamic") {
        return model_has_failures;
    }
    true
}

// LiteLLM's _get_enforced_limit: an absent limit stays absent, and a relaxed one drops to absent
#[must_use]
pub fn get_enforced_limit(
    limit_value: Option<i64>,
    limit_type: Option<&str>,
    model_has_failures: bool,
) -> Option<i64> {
    let value = limit_value?;
    if should_enforce_rate_limit(limit_type, model_has_failures) {
        return Some(value);
    }
    None
}

// LiteLLM's _is_dynamic_rate_limiting_enabled: either limit type being exactly "dynamic" is enough
#[must_use]
// ast-grep-ignore: no-long-names
pub fn is_dynamic_rate_limiting_enabled(
    rpm_limit_type: Option<&str>,
    tpm_limit_type: Option<&str>,
) -> bool {
    rpm_limit_type == Some("dynamic") || tpm_limit_type == Some("dynamic")
}
