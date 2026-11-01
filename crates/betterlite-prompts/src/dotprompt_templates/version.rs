#[must_use]
pub fn strip_version_suffix(prompt_id: &str) -> Option<&str> {
    let (base, version) = prompt_id.rsplit_once(".v")?;
    if base.is_empty() || version.is_empty() || !version.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(base)
}

mod tests;
