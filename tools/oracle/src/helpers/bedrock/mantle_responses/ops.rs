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

pub(super) fn run(input: &Input) -> HelperResult {
    let warnings = Warnings::default();
    let _guard = set_default(tracing_subscriber::registry().with(warnings.clone()));
    match input {
        Input::Url { cases } => Ok(url_cases(cases)),
        Input::ResponseOptions { cases }
        | Input::UnsupportedParams400 { cases }
        | Input::UnsupportedParamsWarnDrop { cases }
        | Input::ToolFilter { cases } => map_cases(&warnings, cases, None),
        Input::ModelName { cases } => {
            let body_input = ResponsesInput::Text("hello".to_owned());
            map_cases(&warnings, cases, Some(&body_input))
        }
        Input::ResponseAuth { cases } | Input::ProjectHeader { cases } => auth_cases(cases),
        Input::Hoisting { cases } => hoist_cases(&warnings, cases),
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
        params: serde_json::from_value(Value::Object(case.params.clone()))?,
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

fn attach_warnings(value: &mut Value, warnings: &[String]) {
    if let Value::Object(fields) = value {
        fields.insert("warnings".to_owned(), json!(warnings));
    }
}

fn body_value(model: &str, input: &ResponsesInput, params: &Object) -> HelperResult {
    Ok(
        match transform_responses_request(model, input, params, None) {
            Ok(body) => json!({"body": serde_json::to_value(body)?}),
            Err(error) => json!({"error": {"status": error.status_code()}}),
        },
    )
}

fn map_cases(
    warnings: &Warnings,
    cases: &[MapCase],
    body_input: Option<&ResponsesInput>,
) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let mut value = match (map_outcome(&runtime, case, warnings)?, body_input) {
            (Err(error), _) => json!({"error": {"status": error.status_code()}}),
            (Ok(params), Some(input)) => body_value(&case.model, input, &params)?,
            (Ok(params), None) => json!({"params": serde_json::to_value(params)?}),
        };
        attach_warnings(&mut value, &warnings.take());
        results.push(value);
    }
    Ok(json!({"results": results}))
}

fn url_cases(cases: &[UrlCase]) -> Value {
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
        urls.push(outcome.map_or_else(|_| json!({"error": true}), |url| json!({"url": url})));
    }
    json!({"urls": urls})
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

fn error_probe(given: Option<&ErrorProbe>) -> Result<Value, Box<HelperError>> {
    let Some(probe) = given else {
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
        warnings.clear();
        let mut value = body_value(&case.model, &request, &case.params)?;
        attach_warnings(&mut value, &warnings.take());
        results.push(value);
    }
    Ok(json!({"results": results}))
}
