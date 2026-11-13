pub mod timeout;

#[path = "env_integer/tests.rs"]
mod tests;

#[must_use]
pub fn get_env_int(value: Option<&str>, default: i64) -> i64 {
    let Some(raw) = value else {
        return default;
    };
    raw.trim().parse().unwrap_or(default)
}

pub fn get_env_int_in_range(
    name: &str,
    value: Option<&str>,
    default: i64,
    minimum: i64,
    maximum: i64,
) -> i64 {
    let parsed = get_env_int(value, default);
    if (minimum..=maximum).contains(&parsed) {
        parsed
    } else {
        tracing::warn!(
            environment_variable = name,
            value = parsed,
            minimum,
            maximum,
            default,
            "integer environment value is outside the supported range; using default"
        );
        default
    }
}

#[must_use]
pub fn get_env_int_or_none(value: Option<&str>) -> Option<i64> {
    let raw = value?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse().ok()
}
