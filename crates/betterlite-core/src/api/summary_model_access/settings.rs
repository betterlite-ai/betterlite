use betterlite_config::sections::summary_model_access::{DEFAULT_SUMMARY_MAX_TOKENS, Settings};

// compact.py imports proxy_server.general_settings inside each reader, so a process without the proxy has none
#[must_use]
pub fn read_summary_model_setting(proxy: Option<&Settings>) -> Option<String> {
    let settings = proxy?;
    settings.context_management_summary_model.clone() // ast-grep-ignore: no-long-names
}

// the fallback is the 4096 constant, never DEFAULT_MAX_TOKENS and never an environment variable
#[must_use]
pub fn read_summary_max_tokens(proxy: Option<&Settings>) -> u64 {
    let Some(settings) = proxy else {
        return DEFAULT_SUMMARY_MAX_TOKENS;
    };
    settings.context_management_summary_max_tokens // ast-grep-ignore: no-long-names
}
