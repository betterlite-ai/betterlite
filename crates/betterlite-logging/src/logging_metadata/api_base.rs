#[must_use]
pub fn strip_trailing_slash(api_base: Option<&str>) -> Option<String> {
    let base = api_base?;
    if base.ends_with("//") {
        return Some(base.trim_end_matches('/').to_owned());
    }
    Some(base.strip_suffix('/').unwrap_or(base).to_owned())
}

mod tests;
