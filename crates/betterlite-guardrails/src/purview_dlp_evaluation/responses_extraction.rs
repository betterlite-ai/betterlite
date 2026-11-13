use betterlite_openai::api::responses_objects::ResponseOutputItem;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::api::responses_objects::sdk::unions::SdkOutput;

fn push_argument(arguments: Option<&str>, args: &mut Vec<String>) {
    if let Some(text) = arguments.filter(|text| !text.trim().is_empty()) {
        args.push(text.to_owned());
    }
}

#[must_use]
pub fn responses_function_call_args(response: &ResponsesApiResponse) -> Vec<String> {
    let mut args = Vec::new();
    for item in &response.output {
        // SdkOutput is the only variant serde builds from the wire, so constructed variants carry nothing to scan.
        let ResponseOutputItem::Sdk(output) = item else {
            continue;
        };
        match output.as_ref() {
            SdkOutput::FunctionCall(call) => {
                push_argument(Some(call.arguments.as_str()), &mut args);
            }
            // A function_call item whose call_id or name is missing keeps its JSON here instead of the typed record.
            SdkOutput::Other(untyped) => {
                let kind = untyped.get("type").and_then(|value| value.as_str());
                let arguments = untyped.get("arguments").and_then(|value| value.as_str());
                if kind == Some("function_call") {
                    push_argument(arguments, &mut args);
                }
            }
            _ => {}
        }
    }
    args
}

mod tests;
