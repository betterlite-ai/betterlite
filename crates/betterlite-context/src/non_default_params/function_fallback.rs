use betterlite_models::controls::json_value::JsonValue;

use super::Params;

pub struct PreProcessedOptional {
    pub optional_params: Params,
    pub add_function_to_prompt: bool,
}

// The cloud providers read these caller keys under a provider-specific name.
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
    let mut add_function_to_prompt = false;
    let sends_functions = ["functions", "function_call", "tools"]
        .iter()
        .any(|name| non_default_params.contains_key(*name));
    if custom_llm_provider == "ollama" && sends_functions {
        optional_params.insert("format".to_owned(), JsonValue::Text("json".to_owned()));
        add_function_to_prompt = true;
        let key = if non_default_params.contains_key("tools") {
            "tools"
        } else {
            "functions"
        };
        if let Some(functions) = non_default_params.remove(key) {
            optional_params.insert("functions_unsupported_model".to_owned(), functions);
            if key == "tools" {
                non_default_params.remove("tool_choice");
            }
        }
    }
    PreProcessedOptional {
        optional_params,
        add_function_to_prompt,
    }
}
