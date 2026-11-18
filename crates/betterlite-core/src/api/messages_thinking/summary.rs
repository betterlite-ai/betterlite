#[must_use]
pub fn auto_summary_enabled(global: bool, environment: Option<&str>) -> bool {
    // the environment only ever turns the global flag on, and only on the exact word "true"
    if global {
        return true;
    }
    environment.is_some_and(|value| value.to_lowercase() == "true")
}
