use betterlite_prompts::anthropic_cache_callbacks::gate::{
    cache_control_custom_logger, should_use_cache_control,
};
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckProbe {
    #[serde(default)]
    params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoggerProbe {
    #[serde(default)]
    params: Object,
    #[serde(default = "default_logger_from_factory")]
    logger_from_factory: bool,
}

const fn default_logger_from_factory() -> bool {
    true
}

pub(super) fn check(probes: &[CheckProbe]) -> Value {
    let results: Vec<bool> = probes
        .iter()
        .map(|probe| should_use_cache_control(&probe.params))
        .collect();
    json!({ "results": results })
}

pub(super) fn logger(probes: &[LoggerProbe]) -> Value {
    let mut results = Vec::with_capacity(probes.len());
    for probe in probes {
        let mut called = false;
        let mut integration = None;
        let logger = cache_control_custom_logger(&probe.params, |name| {
            called = true;
            integration = Some(name);
            probe.logger_from_factory.then_some("custom-logger")
        });
        results.push(json!({
            "called": called,
            "integration": integration,
            "logger": logger,
        }));
    }
    json!({ "results": results })
}
