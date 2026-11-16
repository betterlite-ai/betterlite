use betterlite_core::api::resource_sdk_settings::timeout;
use betterlite_core::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError, TimeoutValue, resolve_fine_tuning_timeout,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let resolved =
            resolve_fine_tuning_timeout(input.timeout.as_ref(), &input.provider, input.structured)
                .map_err(|error| invalid_timeout(&error))?;
        let timeout = match resolved {
            FineTuningTimeout::Seconds(value) => serde_json::json!(value),
            FineTuningTimeout::Structured(value) => serde_json::json!({
                "connect": value.connect, "read": value.read,
                "write": value.write, "pool": value.pool,
            }),
        };
        let request_timeout =
            timeout::request_timeout(input.request_timeout, input.global_request_timeout)
                .map_err(|error| invalid_timeout(&error))?;
        Ok(serde_json::json!({
            "supports_httpx_timeout": (["openai", "azure", "bedrock"].contains(&input.provider.as_str())),
            "timeout": timeout,
            "request_timeout": request_timeout,
        }))
    }
}

fn invalid_timeout(error: &TimeoutError) -> HelperError {
    let message = format!("invalid timeout: {error}");
    HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    }
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct Input {
    provider: String,
    timeout: Option<TimeoutValue>,
    request_timeout: Option<f64>,
    global_request_timeout: Option<f64>,
    structured: Option<StructuredTimeout>,
}
