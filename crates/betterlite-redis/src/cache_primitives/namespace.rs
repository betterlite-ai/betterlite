mod tests;

#[must_use]
pub fn check_and_fix_namespace(namespace: Option<&str>, key: Option<&str>) -> Option<String> {
    let bare_key = key?;
    let Some(active_namespace) = namespace.filter(|value| !value.is_empty()) else {
        return Some(bare_key.to_owned());
    };
    let prefix = format!("{active_namespace}:");
    if bare_key.starts_with(&prefix) {
        Some(bare_key.to_owned())
    } else {
        Some(format!("{prefix}{bare_key}"))
    }
}
