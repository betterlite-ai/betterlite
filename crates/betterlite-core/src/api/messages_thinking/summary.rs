// is_reasoning_auto_summary_enabled: the environment only ever turns the global flag on, on the exact word "true".
#[must_use]
pub fn is_reasoning_auto_summary_enabled(global: bool, environment: Option<&str>) -> bool {
    if global {
        return true;
    }
    environment.is_some_and(|value| value.to_lowercase() == "true")
}
