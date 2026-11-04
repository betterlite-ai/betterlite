use betterlite_server::worker_options::{
    DirectoryMode, LaunchOptions, launch_plan, prometheus_callback_configured, prometheus_directory,
};
use serde::Deserialize;
use std::collections::BTreeMap;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    function: String,
    options: Option<LaunchOptions>,
    workers: Option<u32>,
    settings: Option<BTreeMap<String, serde_json::Value>>,
    metrics_port: Option<u16>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.function.as_str() {
            "launch_plan" => {
                let Some(options) = input.options.as_ref() else {
                    return Err(failure("missing options"));
                };
                match launch_plan(options) {
                    Ok(plan) => serde_json::to_value(plan).map_err(failure),
                    Err(error) => Ok(serde_json::json!({"error": error.to_string()})),
                }
            }
            "callback_configured" => {
                let result = prometheus_callback_configured(input.settings.as_ref());
                Ok(serde_json::Value::Bool(result))
            }
            "prometheus_directory" => {
                let workers = input.workers.unwrap_or(1);
                let directory = prometheus_directory(
                    workers,
                    input.settings.as_ref(),
                    input.metrics_port,
                    &input.environment,
                    DirectoryMode::Plan,
                )
                .map_err(failure)?;
                serde_json::to_value(directory).map_err(failure)
            }
            _ => Err(failure("unknown B4177 function")),
        }
    }
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "WorkerLaunchError".to_owned(),
        message: error.to_string(),
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}
