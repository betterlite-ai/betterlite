use betterlite_bedrock::mantle_inputs::records::MantleGlobals;
use betterlite_bedrock::mantle_responses::auth::{EnvironmentParams, validate_environment};
use betterlite_bedrock::mantle_responses::config::{
    CUSTOM_LLM_PROVIDER, MantleResponsesConfig, SUPPORTS_NATIVE_FILE_SEARCH,
    SUPPORTS_NATIVE_WEBSOCKET,
};
use betterlite_bedrock::mantle_responses::error::{MantleResponsesError, error_from_reply};
use betterlite_bedrock::mantle_responses::options::map_openai_params;
use betterlite_bedrock::mantle_responses::request::transform_responses_request;
use betterlite_bedrock::mantle_responses::url::{CompleteUrlParams, complete_url};
use betterlite_openai::reasoning_models::DropParams;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::params::ResponsesOptions;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::capture::{Fixture, Warnings};
use super::input::{AuthCase, ErrorProbe, HoistCase, Input, MapCase, UrlCase};
use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

const MODEL_NAME_INPUT: &str = "hello";

pub(super) fn run(input: &Input) -> HelperResult {
    let warnings = Warnings::default();
    let _guard = set_default(tracing_subscriber::registry().with(warnings.clone()));
    match input {
        Input::Url { cases } => url_cases(cases),
        Input::ResponseOptions { cases }
        | Input::UnsupportedParams400 { cases }
        | Input::UnsupportedParamsWarnDrop { cases }
        | Input::ToolFilter { cases } => map_cases(&warnings, cases),
        Input::ModelName { cases } => model_name_cases(&warnings, cases),
        Input::ResponseAuth { cases } | Input::ProjectHeader { cases } => auth_cases(cases),
        Input::InputHoisting { cases } => hoist_cases(&warnings, cases),
    }
}

fn config(use_openai_path: Option<bool>) -> MantleResponsesConfig {
    MantleResponsesConfig {
        credential_cache: None,
        use_openai_path: use_openai_path.unwrap_or(true),
    }
}

fn globals(environment: &BTreeMap<String, String>) -> MantleGlobals {
    MantleGlobals {
        mantle_api_key: environment
            .get("BEDROCK_MANTLE_API_KEY")
            .cloned()
            .map(SecretString::from),
        mantle_api_base: environment.get("BEDROCK_MANTLE_API_BASE").cloned(),
        mantle_region: environment.get("BEDROCK_MANTLE_REGION").cloned(),
        aws_bearer_token: environment
            .get("AWS_BEARER_TOKEN_BEDROCK")
            .cloned()
            .map(SecretString::from),
        aws_region_name: environment.get("AWS_REGION_NAME").cloned(),
        aws_region: environment.get("AWS_REGION").cloned(),
    }
}

fn index_headers(source: &BTreeMap<String, String>) -> IndexMap<String, String> {
    source
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn map_outcome(
    runtime: &tokio::runtime::Runtime,
    case: &MapCase,
    warnings: &Warnings,
) -> Result<Result<Object, MantleResponsesError>, Box<HelperError>> {
    let mut fixture = Fixture;
    let mut bundled = BundledReasoningModels::default();
    let options = ResponsesOptions {
        provider: CUSTOM_LLM_PROVIDER,
        model: case.model.clone(),
        params: serde_json::from_value(case.params.clone())?,
    };
    warnings.clear();
    let mapped = runtime.block_on(map_openai_params(
        &config(case.use_openai_path),
        &mut fixture,
        &options,
        DropParams {
            global: false,
            request: case.drop_params,
        },
        &mut bundled,
    ));
    Ok(mapped)
}

fn attach_warnings(value: &mut Value, warnings: Vec<String>) {
    if let Value::Object(fields) = value {
        fields.insert("warnings".to_owned(), json!(warnings));
    }
}

fn run_map(runtime: &tokio::runtime::Runtime, case: &MapCase, warnings: &Warnings) -> HelperResult {
    let mut value = match map_outcome(runtime, case, warnings)? {
        Ok(params) => json!({"params": serde_json::to_value(params)?}),
        Err(error) => json!({"error": {"status": error.status_code()}}),
    };
    attach_warnings(&mut value, warnings.take());
    Ok(value)
}

fn map_cases(warnings: &Warnings, cases: &[MapCase]) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        results.push(run_map(&runtime, case, warnings)?);
    }
    Ok(json!({"results": results}))
}

fn run_body(
    runtime: &tokio::runtime::Runtime,
    case: &MapCase,
    warnings: &Warnings,
) -> HelperResult {
    let mut value = match map_outcome(runtime, case, warnings)? {
        Ok(params) => {
            let input = ResponsesInput::Text(MODEL_NAME_INPUT.to_owned());
            match transform_responses_request(&case.model, &input, &params, None) {
                Ok(body) => json!({"body": serde_json::to_value(body)?}),
                Err(error) => json!({"error": {"status": error.status_code()}}),
            }
        }
        Err(error) => json!({"error": {"status": error.status_code()}}),
    };
    attach_warnings(&mut value, warnings.take());
    Ok(value)
}

fn model_name_cases(warnings: &Warnings, cases: &[MapCase]) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        results.push(run_body(&runtime, case, warnings)?);
    }
    Ok(json!({"results": results}))
}

fn url_cases(cases: &[UrlCase]) -> HelperResult {
    let mut urls = Vec::with_capacity(cases.len());
    for case in cases {
        let params = CompleteUrlParams {
            api_base: case.api_base.as_deref(),
            aws_region_name: case.aws_region_name.as_deref(),
        };
        let outcome = complete_url(
            &config(case.use_openai_path),
            params,
            &globals(&case.environment),
        );
        urls.push(match outcome {
            Ok(url) => json!({"url": url}),
            Err(_) => json!({"error": true}),
        });
    }
    Ok(json!({"urls": urls}))
}

fn auth_cases(cases: &[AuthCase]) -> HelperResult {
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let mut headers = index_headers(&case.headers);
        let api_key = case.api_key.clone().map(SecretString::from);
        validate_environment(
            &mut headers,
            EnvironmentParams {
                api_key: api_key.as_ref(),
                project_id: case.project_id.as_deref(),
            },
            &globals(&case.environment),
        );
        results.push(json!({
            "headers": headers,
            "provider": CUSTOM_LLM_PROVIDER.to_string(),
            "supports_native_file_search": SUPPORTS_NATIVE_FILE_SEARCH,
            "supports_native_websocket": SUPPORTS_NATIVE_WEBSOCKET,
            "error": error_probe(case.error.as_ref())?,
        }));
    }
    Ok(json!({"results": results}))
}

fn error_probe(probe: Option<&ErrorProbe>) -> Result<Value, Box<HelperError>> {
    let Some(probe) = probe else {
        return Ok(Value::Null);
    };
    let pairs: Vec<(String, String)> = probe
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let error = error_from_reply(probe.status, &probe.message, &pairs).or_value_error()?;
    Ok(json!({"status": error.error.status_code, "message": error.error.message}))
}

fn hoist_cases(warnings: &Warnings, cases: &[HoistCase]) -> HelperResult {
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let request: ResponsesInput = serde_json::from_value(case.input.clone())?;
        let params: Object = serde_json::from_value(case.params.clone())?;
        warnings.clear();
        let body = transform_responses_request(&case.model, &request, &params, None);
        let mut value = match body {
            Ok(body) => json!({"body": serde_json::to_value(body)?}),
            Err(error) => json!({"error": {"status": error.status_code()}}),
        };
        attach_warnings(&mut value, warnings.take());
        results.push(value);
    }
    Ok(json!({"results": results}))
}
