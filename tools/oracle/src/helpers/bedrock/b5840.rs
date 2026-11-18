use betterlite_openai::openai_base_config::Headers;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::arguments::json::Object;
use serde_json::{Value, json};

use betterlite_bedrock::sagemaker_runtime::endpoint::{EndpointRequest, runtime_url};
use betterlite_openai::sagemaker_chat::request;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> HelperResult {
        match input.get("function").and_then(Value::as_str) {
            Some("url") => {
                let model = input
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let region = input
                    .get("region")
                    .and_then(Value::as_str)
                    .unwrap_or("us-west-2");
                let base_url = input.get("base_url").and_then(Value::as_str);
                let stream = input.get("stream").and_then(Value::as_bool) == Some(true);
                Ok(json!({
                    "url": runtime_url(&EndpointRequest {
                        model,
                        region,
                        base_url,
                        stream,
                    }),
                }))
            }
            Some("component_header") => {
                let headers: Headers = input
                    .get("headers")
                    .and_then(Value::as_object)
                    .map(|fields| {
                        fields
                            .iter()
                            .filter_map(|(name, value)| {
                                value.as_str().map(|text| (name.clone(), text.to_owned()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mut optional_params = Object::new();
                if let Some(component) = input.get("model_id").and_then(Value::as_str) {
                    optional_params.insert(
                        String::from("model_id"),
                        betterlite_prompts::arguments::json::Json::Text(component.to_owned()),
                    );
                }
                Ok(json!({
                    "headers": request::validate_environment(headers, &optional_params)
                }))
            }
            _ => Ok(json!({"error": "unknown function"})),
        }
    }
}
