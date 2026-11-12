use betterlite_config::sections::deepseek_identity::DEFAULT_API_BASE;

const COMPLETIONS_SUFFIX: &str = "/chat/completions";

#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    let base = api_base
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_API_BASE);
    if base.ends_with(COMPLETIONS_SUFFIX) {
        return base.to_owned();
    }
    format!("{base}{COMPLETIONS_SUFFIX}")
}
