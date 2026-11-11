use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::extra_body::{
    ProviderSpecificRequest, add_provider_specific_params,
};
use betterlite_context::non_default_params::function_fallback::pre_process_optional_params;
use betterlite_context::non_default_params::pre_process::{
    EMBEDDING_ENDPOINT, PreProcessOptions, PreProcessRequest, base_pre_process_params,
    pre_process_non_default_params, remove_sensitive_keys,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::Failure;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RequestInput {
    passed_params: Params,
    special_params: Params,
    custom_llm_provider: String,
    additional_drop_params: Option<Vec<String>>,
}

impl RequestInput {
    fn request(&self) -> PreProcessRequest<'_> {
        PreProcessRequest {
            passed_params: &self.passed_params,
            special_params: &self.special_params,
            custom_llm_provider: &self.custom_llm_provider,
            additional_drop_params: self.additional_drop_params.as_deref().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum PreProcessCall {
    PreProcess {
        request: RequestInput,
        #[serde(default)]
        remove_sensitive_keys: bool,
        #[serde(default)]
        add_provider_specific_params: bool,
    },
    EmbeddingPreProcess {
        request: RequestInput,
    },
}

pub(super) fn run(call: &PreProcessCall) -> Result<Value, Failure> {
    match call {
        PreProcessCall::PreProcess {
            request,
            remove_sensitive_keys,
            add_provider_specific_params,
        } => {
            let options = PreProcessOptions {
                remove_sensitive_keys: *remove_sensitive_keys,
                add_provider_specific_params: *add_provider_specific_params,
            };
            match pre_process_non_default_params(&request.request(), options) {
                Ok(params) => Ok(serde_json::to_value(params)?),
                Err(error) => Err(Failure::Status(error.status_code())),
            }
        }
        PreProcessCall::EmbeddingPreProcess { request } => {
            let processed = base_pre_process_params(&request.request(), &EMBEDDING_ENDPOINT);
            Ok(serde_json::to_value(processed.non_default_params)?)
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum OptionalCall {
    RemoveSensitiveKeys {
        params: Params,
    },
    FunctionFallback {
        passed_params: Params,
        non_default_params: Params,
        custom_llm_provider: String,
    },
    AddProviderSpecific {
        optional_params: Params,
        passed_params: Params,
        custom_llm_provider: String,
        openai_params: Vec<String>,
        additional_drop_params: Option<Vec<String>>,
    },
}

pub(super) fn run_optional(call: &OptionalCall) -> Result<Value, Failure> {
    match call {
        OptionalCall::RemoveSensitiveKeys { params } => {
            let mut kept = params.clone();
            remove_sensitive_keys(&mut kept);
            Ok(serde_json::to_value(kept)?)
        }
        OptionalCall::FunctionFallback {
            passed_params,
            non_default_params,
            custom_llm_provider,
        } => {
            let mut remaining = non_default_params.clone();
            let processed =
                pre_process_optional_params(passed_params, &mut remaining, custom_llm_provider);
            Ok(json!({
                "optional_params": processed.optional_params,
                "non_default_params": remaining,
                "add_function_to_prompt": processed.add_function_to_prompt,
            }))
        }
        OptionalCall::AddProviderSpecific {
            optional_params,
            passed_params,
            custom_llm_provider,
            openai_params,
            additional_drop_params,
        } => {
            let names: Vec<&str> = openai_params.iter().map(String::as_str).collect();
            let request = ProviderSpecificRequest {
                passed_params,
                custom_llm_provider,
                openai_params: &names,
                additional_drop_params: additional_drop_params.as_deref().unwrap_or_default(),
            };
            match add_provider_specific_params(optional_params.clone(), &request) {
                Ok(params) => Ok(serde_json::to_value(params)?),
                Err(error) => Err(Failure::Status(error.status_code())),
            }
        }
    }
}
