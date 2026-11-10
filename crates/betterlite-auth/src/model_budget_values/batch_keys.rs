mod tests;

#[must_use]
// ast-grep-ignore: no-one-line-functions
pub fn batch_charged_once_marker_key(spend_key: &str, batch_id: &str) -> String {
    format!("{spend_key}:batch:{batch_id}")
}
