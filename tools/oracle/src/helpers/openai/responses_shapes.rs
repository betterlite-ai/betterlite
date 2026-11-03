pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        use betterlite_openai::api::responses_shapes::WsRequestDefaults;
        use betterlite_openai::api::responses_shapes::builders::{
            build_interpreter_logs, build_web_search_call,
        };
        let operation = input
            .get("operation")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        match operation {
            "web_search_sources" => serde_json::to_value(build_web_search_call(
                input.get("tool_id").and_then(serde_json::Value::as_str).unwrap_or_default(),
                input.get("tool_input").unwrap_or(&serde_json::Value::Null),
                input.get("result").unwrap_or(&serde_json::Value::Null),
                input.get("status").and_then(serde_json::Value::as_str),
            )),
            "interpreter_logs" => serde_json::to_value(build_interpreter_logs(input.get("content").unwrap_or(&serde_json::Value::Null))),
            "websocket_defaults" => {
                let defaults: WsRequestDefaults = serde_json::from_value(input.clone()).map_err(|error| Box::new(crate::helpers::HelperError { class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string() }))?;
                let request = input.get("request").and_then(serde_json::Value::as_object).cloned().unwrap_or_default();
                serde_json::to_value(defaults.merged_into(&request))
            }
            "nullable_records" => {
                use betterlite_openai::api::responses_shapes::{DeleteResponseResult, OutputCodeInterpreterCall, OutputFunctionToolCall, OutputImageGenerationCall, OutputText};
                let decode = |name: &str| input.get(name).cloned().ok_or_else(|| "missing record".to_owned());
                let result = (|| -> Result<serde_json::Value, String> {
                    let deleted: DeleteResponseResult = serde_json::from_value(decode("record")?).map_err(|error| error.to_string())?;
                    let output_text: OutputText = serde_json::from_value(decode("output_text")?).map_err(|error| error.to_string())?;
                    let function_call: OutputFunctionToolCall = serde_json::from_value(decode("function_call")?).map_err(|error| error.to_string())?;
                    let image_call: OutputImageGenerationCall = serde_json::from_value(decode("image_call")?).map_err(|error| error.to_string())?;
                    let interpreter_call: OutputCodeInterpreterCall = serde_json::from_value(decode("interpreter_call")?).map_err(|error| error.to_string())?;
                    Ok(serde_json::json!({"deleted":deleted,"output_text":output_text,"function_call":function_call,"image_call":image_call,"interpreter_call":interpreter_call}))
                })();
                result.map_err(|message| serde_json::Error::io(std::io::Error::other(message)))
            }
            _ => return Err(Box::new(crate::helpers::HelperError { class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None, message: "unknown responses_shapes operation".to_owned() })),
        }.map_err(|error| Box::new(crate::helpers::HelperError { class: "InvalidInput".to_owned(), status: None, error_type: None, param: None, code: None, message: error.to_string() }))
    }
}
