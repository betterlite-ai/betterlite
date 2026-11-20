use betterlite_config::loader::EnvironmentLookup;

pub const CALLBACK_NAME: &str = "arize_phoenix";

// litellm_logging._maybe_auto_initialize_arize_phoenix: any Phoenix variable turns the auto start on
#[must_use]
pub fn configured(env: &dyn EnvironmentLookup) -> bool {
    [
        "PHOENIX_API_KEY",
        "PHOENIX_COLLECTOR_HTTP_ENDPOINT",
        "PHOENIX_COLLECTOR_ENDPOINT",
    ]
    .iter()
    .any(|name| env.lookup(name).is_some_and(|value| !value.is_empty()))
}
