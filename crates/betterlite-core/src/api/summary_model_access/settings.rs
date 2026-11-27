use betterlite_config::sections::summary_model_access::{DEFAULT_SUMMARY_MAX_TOKENS, Settings};

// compact.py imports proxy_server inside each reader: without it, null and the 4096 constant, never DEFAULT_MAX_TOKENS
#[must_use]
pub fn read_summary_settings(proxy: Option<&Settings>) -> (Option<String>, u64) {
    let Some(settings) = proxy else {
        return (None, DEFAULT_SUMMARY_MAX_TOKENS);
    };
    (
        // ast-grep-ignore: no-long-names
        settings.context_management_summary_model.clone(),
        // ast-grep-ignore: no-long-names
        settings.context_management_summary_max_tokens,
    )
}
