use betterlite_config::sections::guardrail_params::params::LitellmParams;

use super::callback::{CallbackScoping, GuardrailCallback};
use super::error::RegistryError;

pub fn configure_scoping(
    callback: &mut GuardrailCallback,
    guardrail_name: &str,
    params: &LitellmParams,
    global_skip_tool_message: bool,
) -> Result<(), RegistryError> {
    let fields = &params.base.fields;
    callback.scoping = CallbackScoping {
        skip_system_message: fields.skip_system_message,
        skip_tool_message: fields.skip_tool_message_in_guardrail,
        scan_only_tool_results: fields.scan_only_tool_results,
    };
    let scan_only_tool_results = callback.scoping.scan_only_tool_results == Some(true);
    if scan_only_tool_results && !callback.supports_tool_result_scan() {
        return Err(RegistryError::ScanOnlyUnsupported {
            guardrail_name: guardrail_name.to_owned(),
        });
    }
    let skip_tool_message = callback
        .scoping
        .skip_tool_message
        .unwrap_or(global_skip_tool_message);
    if scan_only_tool_results && skip_tool_message {
        return Err(RegistryError::ScanOnlyWithSkipTool {
            guardrail_name: guardrail_name.to_owned(),
        });
    }
    // the constructor's own default stays when the params leave these out
    if let Some(run_in_parallel) = fields.run_in_parallel {
        callback.base.settings.run_in_parallel = run_in_parallel;
    }
    if let Some(scan_raw_request) = fields.scan_raw_request {
        callback.base.settings.scan_raw_request = scan_raw_request;
    }
    Ok(())
}
