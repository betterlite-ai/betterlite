use std::collections::BTreeMap;

use betterlite_bedrock::responses::config::{
    BedrockResponsesConfig, CUSTOM_LLM_PROVIDER, SUPPORTS_NATIVE_FILE_SEARCH,
};
use betterlite_bedrock::responses::error::ResponsesError;
use betterlite_bedrock::responses::images::{collect_remote_image_urls, inline_remote_image_urls};
use betterlite_bedrock::responses::options::map_openai_params;
use betterlite_bedrock::responses::request::{
    async_transform_responses, transform_inlined_request, transform_responses_request,
};
use betterlite_bedrock::responses::url::{CompleteUrlParams, complete_url};
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_openai::reasoning_models::DropParams;
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::capture::{Fixture, MapFetcher, Warnings};
use super::input::{CodexCase, FetchCase, Input, OptionCase, ReplacementCase, RouteCase, UrlCase};
use super::sign::{auth_case, sigv4_case};

pub(super) const API_BASE: &str =
    "https://bedrock-runtime.us-west-2.amazonaws.com/openai/v1/responses";
const MODEL: &str = "openai.gpt-oss-120b";

pub(super) type Case = Result<Value, Box<HelperError>>;

pub(super) fn run(input: &Input) -> HelperResult {
    let warnings = Warnings::default();
    let _guard = set_default(tracing_subscriber::registry().with(warnings.clone()));
    let results: Vec<Value> = match input {
        Input::BedrockResponsesModelRoute(cases) => {
            cases.iter().map(route_case).collect::<Result<_, _>>()?
        }
        Input::ResponsesBearerAuth(cases) => {
            cases.iter().map(auth_case).collect::<Result<_, _>>()?
        }
        Input::ResponsesSigv4(cases) => cases.iter().map(sigv4_case).collect::<Result<_, _>>()?,
        Input::ResponseOptionGates(cases) | Input::NativeFileSearchFlag(cases) => cases
            .iter()
            .map(|case| recorded(&warnings, || option_case(case)))
            .collect::<Result<_, _>>()?,
        Input::InlineImageTraversal(cases) => cases
            .iter()
            .map(|case| json!(collect_remote_image_urls(&case.input)))
            .collect(),
        Input::RemoteUrlInlined(cases) => cases.iter().map(fetch_case).collect::<Result<_, _>>()?,
        Input::ResponsesRuntimeUrl(cases) => cases.iter().map(url_case).collect(),
        Input::InlineImageReplacement(cases) => cases
            .iter()
            .map(replacement_case)
            .collect::<Result<_, _>>()?,
        Input::CodexInputRewrite(cases) => cases
            .iter()
            .map(|case| recorded(&warnings, || codex_case(case)))
            .collect::<Result<_, _>>()?,
    };
    Ok(json!({"results": results}))
}

pub(super) fn runtime() -> Result<Runtime, Box<HelperError>> {
    let mut builder = Builder::new_current_thread();
    builder.enable_all();
    builder.build().or_value_error()
}

// LiteLLM's warnings are part of the answer, so the log is emptied first and read after the case ran
fn recorded<Run>(warnings: &Warnings, run_case: Run) -> Case
where
    Run: FnOnce() -> Case,
{
    drop(warnings.take());
    let mut value = run_case()?;
    if let Value::Object(fields) = &mut value {
        fields.insert("warnings".to_owned(), json!(warnings.take()));
    }
    Ok(value)
}

fn route_case(case: &RouteCase) -> Case {
    let fetcher = MapFetcher::new(&BTreeMap::new());
    let config =
        BedrockResponsesConfig::for_model(case.model.as_deref(), &case.model_cost, fetcher)
            .or_value_error()?;
    let selected = config.is_some();
    Ok(json!({
        "selected": selected,
        "provider": selected.then(|| CUSTOM_LLM_PROVIDER.to_string()),
        "native_file_search": selected.then_some(SUPPORTS_NATIVE_FILE_SEARCH),
    }))
}

fn option_case(case: &OptionCase) -> Case {
    let drop_params = DropParams {
        global: false,
        request: case.drop_params,
    };
    let mapped = runtime()?
        .block_on(map_openai_params(
            &mut Fixture,
            MODEL,
            &case.params,
            drop_params,
            &mut BundledReasoningModels::default(),
        ))
        .or_value_error()?;
    Ok(json!({
        "params": serde_json::to_value(mapped).or_value_error()?,
        "native_file_search": SUPPORTS_NATIVE_FILE_SEARCH,
    }))
}

fn replacement_case(case: &ReplacementCase) -> Case {
    let Ok(replaced) = inline_remote_image_urls(&case.input, &case.inlined) else {
        return Ok(json!({"error": true}));
    };
    Ok(json!({"input": serde_json::to_value(replaced).or_value_error()?}))
}

fn fetch_outcome(case: &FetchCase, asynchronous: bool) -> Case {
    let config = BedrockResponsesConfig {
        fetcher: MapFetcher::new(&case.data_urls),
    };
    let params = Object::new();
    let running = runtime()?;
    let outcome = if asynchronous {
        running.block_on(async_transform_responses(
            &config.fetcher,
            MODEL,
            &case.input,
            &params,
            None,
        ))
    } else {
        running.block_on(transform_responses_request(
            &config.fetcher,
            MODEL,
            &case.input,
            &params,
            None,
        ))
    };
    let fetched = config.fetcher.fetched();
    match outcome {
        Ok(body) => Ok(json!({
            "body": serde_json::to_value(body).or_value_error()?,
            "fetched": fetched,
        })),
        Err(ResponsesError::Fetch(_)) => Ok(json!({"error": "fetch", "fetched": fetched})),
        Err(error) => Err(error).or_value_error(),
    }
}

fn fetch_case(case: &FetchCase) -> Case {
    Ok(json!({
        "sync": fetch_outcome(case, false)?,
        "async": fetch_outcome(case, true)?,
    }))
}

fn url_case(case: &UrlCase) -> Value {
    let environment = |name: &str| case.environment.get(name).map(String::as_str);
    let outcome = complete_url(CompleteUrlParams {
        api_base: case.api_base.as_deref(),
        aws_region_name: case.litellm_params.aws_region_name.as_deref(),
        aws_bedrock_runtime_endpoint: case.litellm_params.aws_bedrock_runtime_endpoint.as_deref(),
        environment_endpoint: environment("AWS_BEDROCK_RUNTIME_ENDPOINT"),
        region_inputs: RegionInputs {
            environment_region_name: environment("AWS_REGION_NAME"),
            environment_region: environment("AWS_REGION"),
            session_region: None,
        },
    });
    outcome.map_or_else(|_| json!({"error": true}), |url| json!({"url": url}))
}

fn codex_case(case: &CodexCase) -> Case {
    let body =
        transform_inlined_request(&case.model, &case.input, &case.params, None).or_value_error()?;
    Ok(json!({
        "body": serde_json::to_value(body).or_value_error()?,
    }))
}
