use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_reporting::safe_proxy_config::{ProxyConfigSnapshot, safe_config_lines};

pub(crate) struct Op;

impl Helper for Op {
    type Input = ProxyConfigSnapshot;

    fn run(input: &Self::Input) -> HelperResult {
        let lines = safe_config_lines(&input.config, &input.general_settings);
        Ok(serde_json::Value::Array(
            lines.into_iter().map(serde_json::Value::String).collect(),
        ))
    }
}
