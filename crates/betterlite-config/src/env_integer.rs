#[path = "env_integer/tests.rs"]
mod tests;

pub fn get_env_int(name: &str, value: Option<&str>, default: i64) -> i64 {
    let Some(value) = value else {
        return default;
    };
    match value.trim().parse::<i64>() {
        Ok(value) => value,
        Err(_) => {
            tracing::warn!(
                environment_variable = name,
                "invalid integer environment value; using default"
            );
            default
        }
    }
}

pub fn get_env_int_in_range(
    name: &str,
    value: Option<&str>,
    default: i64,
    minimum: i64,
    maximum: i64,
) -> i64 {
    let parsed = get_env_int(name, value, default);
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

pub fn get_env_int_or_none(value: Option<&str>) -> Option<i64> {
    let Some(value) = value else {
        return None;
    };
    match value.trim().parse::<i64>() {
        Ok(value) => Some(value),
        Err(_) => None,
    }
}
