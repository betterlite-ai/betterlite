use betterlite_core::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, TimeoutError, resolve_fine_tuning_timeout, supports_httpx_timeout,
};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
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
            betterlite_core::api::resource_sdk_settings::timeout::request_timeout(
                input.request_timeout,
                input.global_request_timeout,
            )
            .map_err(|error| invalid_timeout(&error))?;
        Ok(serde_json::json!({
            "supports_httpx_timeout": supports_httpx_timeout(&input.provider),
            "timeout": timeout,
            "request_timeout": request_timeout,
        }))
    }
}

fn invalid_timeout(error: &TimeoutError) -> crate::helpers::HelperError {
    let message = format!("invalid timeout: {error}");
    crate::helpers::HelperError {
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
    timeout: Option<betterlite_core::api::resource_sdk_settings::timeout::TimeoutValue>,
    request_timeout: Option<f64>,
    global_request_timeout: Option<f64>,
    structured: Option<betterlite_core::api::resource_sdk_settings::timeout::StructuredTimeout>,
}
