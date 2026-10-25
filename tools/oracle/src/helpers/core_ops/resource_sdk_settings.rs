pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let supports =
            betterlite_core::api::resource_sdk_settings::supports_httpx_timeout(&input.provider);
        let resolved_timeout =
            betterlite_core::api::resource_sdk_settings::resolve_fine_tuning_timeout(
                input.timeout.as_deref(),
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
        let timeout_json = match resolved_timeout {
            betterlite_core::api::resource_sdk_settings::FineTuningTimeout::Seconds(value) => {
                serde_json::json!(value)
            }
            betterlite_core::api::resource_sdk_settings::FineTuningTimeout::Structured(value) => {
                serde_json::json!({
                    "connect": value.connect,
                    "read": value.read,
                    "write": value.write,
                    "pool": value.pool,
                })
            }
        };
        Ok(serde_json::json!({
            "supports_httpx_timeout": supports,
            "timeout": timeout_json,
            "request_timeout": betterlite_core::api::resource_sdk_settings::request_timeout(
                input.request_timeout,
                input.global_request_timeout,
            ),
        }))
    }
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct Input {
    provider: String,
    timeout: Option<String>,
    request_timeout: Option<f64>,
    global_request_timeout: Option<f64>,
    structured: Option<betterlite_core::api::resource_sdk_settings::StructuredTimeout>,
}
