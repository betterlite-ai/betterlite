mod input;
mod operations;
mod support;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use self::input::Input;
use self::operations::{
    append_output, call_parameter_streaming, provider_call_error, request_params,
    tool_execution_events,
};
use self::support::Settings;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::RequestParams { scenarios } => request_params(scenarios),
            Input::ToolExecutionEvents {
                request,
                tool_calls,
                tool_results,
                ids,
            } => tool_execution_events(request, tool_calls, tool_results, ids),
            Input::CallParameterStreaming {
                request,
                call_params,
                should_auto_execute,
                original_stream,
                ids,
            } => call_parameter_streaming(
                request,
                call_params,
                Settings {
                    should_auto_execute: *should_auto_execute,
                    original_stream: *original_stream,
                },
                ids,
            ),
            Input::AppendOutput {
                response,
                mcp_tools_fetched,
                tool_results,
                ids,
            } => append_output(response, mcp_tools_fetched, tool_results, ids),
            Input::ProviderCallError {
                request,
                call_params,
                follow_up_input,
                status,
                ids,
            } => provider_call_error(request, call_params, follow_up_input, *status, ids),
        }
    }
}
