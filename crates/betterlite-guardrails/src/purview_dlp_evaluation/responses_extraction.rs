use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseOutputItem;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::api::responses_objects::sdk::unions::SdkOutput;

fn push_argument(arguments: Option<&str>, args: &mut Vec<String>) {
    if let Some(text) = arguments.filter(|text| !text.trim().is_empty()) {
        args.push(text.to_owned());
    }
}

fn push_sdk_arguments(output: &SdkOutput, args: &mut Vec<String>) {
    match output {
        SdkOutput::FunctionCall(call) => push_argument(Some(call.arguments.as_str()), args),
        SdkOutput::OutputMessage(_)
        | SdkOutput::FileSearch(_)
        | SdkOutput::FunctionCallOutput(_)
        | SdkOutput::WebSearch(_)
        | SdkOutput::Computer(_)
        | SdkOutput::ComputerOutput(_)
        | SdkOutput::Reasoning(_)
        | SdkOutput::Program(_)
        | SdkOutput::ProgramOutput(_)
        | SdkOutput::SearchCall(_)
        | SdkOutput::SearchOutput(_)
        | SdkOutput::AdditionalTools(_)
        | SdkOutput::Compaction(_)
        | SdkOutput::ImageCall(_)
        | SdkOutput::CodeInterpreterCall(_)
        | SdkOutput::LocalShellCall(_)
        | SdkOutput::LocalShellCallOutput(_)
        | SdkOutput::OutputShell(_)
        | SdkOutput::ShellOutput(_)
        | SdkOutput::OutputApplyPatch(_)
        | SdkOutput::ApplyPatchOutput(_)
        | SdkOutput::McpCall(_)
        | SdkOutput::McpListTools(_)
        | SdkOutput::McpApprovalRequest(_)
        | SdkOutput::McpApprovalResponse(_)
        | SdkOutput::CustomCall(_)
        | SdkOutput::CustomCallOutput(_) => {}
    }
}

#[must_use]
pub fn responses_function_call_args(response: &ResponsesApiResponse) -> Vec<String> {
    let mut args = Vec::new();
    for item in &response.output {
        match item {
            ResponseOutputItem::Sdk(output) => push_sdk_arguments(output, &mut args),
            ResponseOutputItem::FunctionCall(call) => {
                if call.r#type.as_deref() == Some("function_call") {
                    push_argument(call.arguments.as_deref(), &mut args);
                }
            }
            // A function_call item whose call_id or name is missing fails the typed records and lands here.
            ResponseOutputItem::Unknown(untyped) => {
                if let Some(Json::Text(kind)) = untyped.get("type")
                    && kind == "function_call"
                    && let Some(Json::Text(arguments)) = untyped.get("arguments")
                {
                    push_argument(Some(arguments.as_str()), &mut args);
                }
            }
            ResponseOutputItem::Message(_)
            | ResponseOutputItem::ImageGeneration(_)
            | ResponseOutputItem::CodeInterpreter(_)
            | ResponseOutputItem::CustomTool(_) => {}
        }
    }
    args
}

mod tests;
