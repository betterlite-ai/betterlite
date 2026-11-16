use betterlite_foundry::image_generation::config::FoundryImageConfig;
use betterlite_foundry::image_generation::error::FoundryImageError;
use betterlite_foundry::image_generation::flux::{
    flux2_image_generation_url, flux2_provider_model_path, is_flux2_model, supported_params,
};
use betterlite_foundry::image_generation::params::map_openai_params;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Serialize;
use serde_json::{Map, Value, json};

mod input;
mod tests;

use input::{Input, ParamsCall, UrlCall};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answer = Map::new();
        if let Some(models) = &input.models {
            let classes: Vec<&str> = models
                .iter()
                .map(|model| selected_class(FoundryImageConfig::select(model)))
                .collect();
            answer.insert("models".to_owned(), json!(classes));
        }
        if let Some(models) = &input.flux2 {
            let detected: Vec<bool> = models.iter().map(|model| is_flux2_model(model)).collect();
            answer.insert("flux2".to_owned(), json!(detected));
        }
        if let Some(models) = &input.paths {
            let paths: Vec<&str> = models
                .iter()
                .map(|model| flux2_provider_model_path(model))
                .collect();
            answer.insert("paths".to_owned(), json!(paths));
        }
        if let Some(urls) = &input.urls {
            let outcomes: Vec<Value> = urls.iter().map(url_outcome).collect();
            answer.insert("urls".to_owned(), json!(outcomes));
        }
        if let Some(models) = &input.supported {
            let allowlists: Vec<Value> = models
                .iter()
                .map(|model| json!(supported_params(model)))
                .collect();
            answer.insert("supported".to_owned(), json!(allowlists));
        }
        if let Some(params) = &input.params {
            let outcomes: Vec<Value> = params.iter().map(params_outcome).collect();
            answer.insert("params".to_owned(), json!(outcomes));
        }
        Ok(Value::Object(answer))
    }
}

// The LiteLLM config class each FoundryImageConfig stands for, so the Python adapter's class name compares equal
const fn selected_class(config: FoundryImageConfig) -> &'static str {
    match config {
        FoundryImageConfig::DallE2 => "AzureFoundryDallE2ImageGenerationConfig",
        FoundryImageConfig::DallE3 => "AzureFoundryDallE3ImageGenerationConfig",
        FoundryImageConfig::Mai => "AzureFoundryMAIImageGenerationConfig",
        FoundryImageConfig::Flux => "AzureFoundryFluxImageGenerationConfig",
        FoundryImageConfig::GptImage => "AzureFoundryGPTImageGenerationConfig",
    }
}

// The LiteLLM exception each error variant stands for; OpenAIError never covers these
const fn error_kind(error: &FoundryImageError) -> &'static str {
    match error {
        FoundryImageError::InvalidSize { .. } => "BadRequestError",
        FoundryImageError::UnsupportedParams { .. } => "UnsupportedParamsError",
        FoundryImageError::ApiBaseRequired
        | FoundryImageError::WholeNumber { .. }
        | FoundryImageError::Decimal { .. }
        | FoundryImageError::GptParams(_) => "ValueError",
    }
}

fn failure(error: &FoundryImageError) -> Value {
    json!({
        "error": {
            "kind": error_kind(error),
            "status": error.status_code(),
            "provider": error.llm_provider(),
        }
    })
}

fn outcome<T: Serialize>(result: Result<T, FoundryImageError>) -> Value {
    match result {
        Ok(value) => json!({ "value": value }),
        Err(error) => failure(&error),
    }
}

fn url_outcome(call: &UrlCall) -> Value {
    outcome(flux2_image_generation_url(
        call.api_base.as_deref(),
        &call.model,
        call.api_version.as_deref(),
    ))
}

fn params_outcome(call: &ParamsCall) -> Value {
    outcome(map_openai_params(
        &call.non_default_params,
        &call.optional_params,
        &call.model,
        call.drop_params,
    ))
}
