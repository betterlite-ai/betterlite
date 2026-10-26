pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let supports =
            betterlite_core::api::resource_sdk_settings::supports_httpx_timeout(&input.provider);
        let timeout_json = timeout_json(input)?;
        let request_timeout = request_timeout(input)?;
        Ok(serde_json::json!({
            "supports_httpx_timeout": supports,
            "timeout": timeout_json,
            "request_timeout": request_timeout,
        }))
    }
}

fn timeout_json(input: &Input) -> crate::helpers::HelperResult {
    let resolved = betterlite_core::api::resource_sdk_settings::resolve_fine_tuning_timeout(
        input.timeout.as_ref(),
        &input.provider,
        input.structured,
    )
    .map_err(|error| crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: Some(Some("timeout".to_owned())),
        code: None,
        message: format!("invalid timeout: {error:?}"),
    })?;
    let value = match resolved {
        betterlite_core::api::resource_sdk_settings::FineTuningTimeout::Seconds(value) => {
            serde_json::json!(value)
        }
        betterlite_core::api::resource_sdk_settings::FineTuningTimeout::Structured(value) => {
            serde_json::json!({"connect": value.connect, "read": value.read, "write": value.write, "pool": value.pool})
        }
    };
    Ok(value)
}

fn request_timeout(input: &Input) -> crate::helpers::HelperResult {
    let value = betterlite_core::api::resource_sdk_settings::request_timeout(
        input.request_timeout,
        input.global_request_timeout,
    )
    .map(|value| serde_json::json!(value))
    .map_err(|error| crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: Some(Some("timeout".to_owned())),
        code: None,
        message: format!("invalid timeout: {error}"),
    })?;
    Ok(serde_json::json!(value))
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct Input {
    provider: String,
    timeout: Option<betterlite_core::api::resource_sdk_settings::TimeoutValue>,
    request_timeout: Option<f64>,
    global_request_timeout: Option<f64>,
    structured: Option<betterlite_core::api::resource_sdk_settings::StructuredTimeout>,
}
