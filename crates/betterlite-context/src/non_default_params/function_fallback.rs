use betterlite_models::controls::json_value::JsonValue;

use super::Params;

pub struct PreProcessedOptional {
    pub optional_params: Params,
    pub add_function_to_prompt: bool,
}

fn special_auth_names(custom_llm_provider: &str) -> &'static [(&'static str, &'static str)] {
    match custom_llm_provider {
        "azure" => &[("token", "azure_ad_token")],
        "bedrock" => &[("region_name", "aws_region_name")],
        "vertex_ai" | "vertex_ai_beta" => &[
            ("project", "vertex_project"),
            ("region_name", "vertex_location"),
        ],
        "watsonx" => &[
            ("project", "watsonx_project"),
            ("region_name", "watsonx_region_name"),
            ("token", "watsonx_token"),
        ],
        _ => &[],
    }
}

// LiteLLM's guard names ollama first and then lists providers that can never equal it, so ollama is the only provider that reaches the fallback and no provider raises.
pub fn pre_process_optional_params(
    passed_params: &Params,
    non_default_params: &mut Params,
    custom_llm_provider: &str,
) -> PreProcessedOptional {
    let mut optional_params = Params::new();
    for (param, name) in special_auth_names(custom_llm_provider) {
        if let Some(value) = passed_params.get(*param) {
            optional_params.insert((*name).to_owned(), value.clone());
        }
    }
    let sends_functions = ["functions", "function_call", "tools"]
        .iter()
        .any(|name| non_default_params.contains_key(*name));
    let add_function_to_prompt = custom_llm_provider == "ollama" && sends_functions;
    if add_function_to_prompt {
        optional_params.insert("format".to_owned(), JsonValue::Text("json".to_owned()));
        let moved = if let Some(tools) = non_default_params.remove("tools") {
            non_default_params.remove("tool_choice");
            Some(tools)
        } else {
            non_default_params.remove("functions")
        };
        if let Some(functions) = moved {
            optional_params.insert("functions_unsupported_model".to_owned(), functions);
        }
    }
    PreProcessedOptional {
        optional_params,
        add_function_to_prompt,
    }
}
