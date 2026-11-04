use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    address: String,
    #[serde(default)]
    enabled: bool,
    job_role: Option<String>,
    suppress: Option<String>,
    debug: bool,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let address = match betterlite_config::collector::parse_collector_address(&input.address) {
            Ok(betterlite_config::collector::CollectorAddress::Unix { path }) => {
                json!({"kind":"unix", "path":path})
            }
            Ok(betterlite_config::collector::CollectorAddress::Tcp { host, port }) => {
                json!({"kind":"tcp", "host":host, "port":port})
            }
            Err(error) => json!({"error":error.to_string()}),
        };
        let suppress = betterlite_logging::spend_error::loads_spend_traceback_policy(
            input.suppress.as_deref(),
        ) && !input.debug;
        Ok(
            json!({"address":address,"produces":input.enabled && input.job_role.as_deref() != Some("collector"),"suppress":suppress}),
        )
    }
}
