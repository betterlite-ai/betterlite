use std::collections::BTreeMap;
use std::future::ready;
use std::time::{Duration, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::responses::auth::{SignInputs, sign_request, validate_environment};
use betterlite_bedrock::responses::config::{
    BedrockResponsesConfig, CUSTOM_LLM_PROVIDER, SUPPORTS_NATIVE_FILE_SEARCH,
};
use betterlite_bedrock::responses::error::{ResponsesError, error_from_reply};
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
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::capture::{Fixture, MapFetcher, Warnings};
use super::input::{
    AuthCase, CodexCase, ErrorProbe, FetchCase, Input, OptionCase, ReplacementCase, RouteCase,
    SigningCase, UrlCase,
};

const API_BASE: &str = "https://bedrock-runtime.us-west-2.amazonaws.com/openai/v1/responses";
const MODEL: &str = "openai.gpt-oss-120b";
const CLOCK_SECONDS: u64 = 1_767_225_600;
const BEARER_VARIABLE: &str = "AWS_BEARER_TOKEN_BEDROCK";

type Case = Result<Value, Box<HelperError>>;

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
            .map(|case| option_case(&warnings, case))
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
            .map(|case| codex_case(&warnings, case))
            .collect::<Result<_, _>>()?,
    };
    Ok(json!({"results": results}))
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    let mut builder = Builder::new_current_thread();
    builder.enable_all();
    builder.build().or_value_error()
}

fn secret_of(text: Option<&String>) -> Option<SecretString> {
    text.cloned().map(SecretString::from)
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

fn auth_case(case: &AuthCase) -> Case {
    let api_key = secret_of(case.api_key.as_ref());
    let environment_token = secret_of(case.environment.get(BEARER_VARIABLE));
    let mut validated = case.headers.clone();
    validate_environment(&mut validated, api_key.as_ref(), environment_token.as_ref());
    Ok(json!({"headers": validated}))
}

fn error_value(probe: Option<&ErrorProbe>) -> Case {
    let Some(given) = probe else {
        return Ok(Value::Null);
    };
    let pairs: Vec<(String, String)> = given
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let error = error_from_reply(given.status, &given.message, &pairs).or_value_error()?;
    let request_id = error
        .response_headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-amzn-requestid"))
        .map(|(_, value)| value.clone());
    Ok(json!({
        "status": error.error.status_code,
        "message": error.error.message,
        "request_id": request_id,
    }))
}

// The signing time and signature change on every run, so only their shape is compared
fn signed_summary(headers: &BTreeMap<String, Option<String>>) -> Case {
    let text = |name: &str| headers.get(name).cloned().flatten().unwrap_or_default();
    let authorization = text("Authorization");
    let (_, rest) = authorization
        .split_once("Credential=")
        .ok_or("no credential scope")
        .or_value_error()?;
    let (credential, rest) = rest
        .split_once(", SignedHeaders=")
        .ok_or("no signed headers")
        .or_value_error()?;
    let (signed_headers, _signature) = rest
        .split_once(", Signature=")
        .ok_or("no signature")
        .or_value_error()?;
    let fields: Vec<&str> = credential.split('/').collect();
    let [access_key, _date, region, service, _scope] = fields.as_slice() else {
        return Err("the credential scope has the wrong shape").or_value_error();
    };
    let plain: BTreeMap<&String, &Option<String>> = headers
        .iter()
        .filter(|(name, _)| !["Authorization", "X-Amz-Date"].contains(&name.as_str()))
        .collect();
    Ok(json!({
        "access_key": access_key,
        "region": region,
        "service": service,
        "signed_headers": signed_headers,
        "headers": plain,
    }))
}

fn sigv4_case(case: &SigningCase) -> Case {
    let headers: Vec<(String, Option<String>)> = case
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Some(value.clone())))
        .collect();
    let inputs = SignInputs {
        headers: &headers,
        optional_params: &case.optional_params,
        request_data: &case.request_data,
        api_base: API_BASE,
        model: None,
        api_key: None,
        environment_token: None,
        region_inputs: RegionInputs::default(),
        now: UNIX_EPOCH
            .checked_add(Duration::from_secs(CLOCK_SECONDS))
            .ok_or("the clock is out of range")
            .or_value_error()?,
    };
    let signed = runtime()?
        .block_on(sign_request(inputs, |settings, _region| {
            ready(
                match (&settings.access_key_id, &settings.secret_access_key) {
                    (Some(access), Some(secret)) => Ok(Credentials {
                        access_key_id: access.clone(),
                        secret_access_key: secret.clone(),
                        session_token: None,
                    }),
                    _ => Err(CredentialError::MissingCredentials),
                },
            )
        }))
        .or_value_error()?;
    let signed_headers: BTreeMap<String, Option<String>> = signed.headers.into_iter().collect();
    Ok(json!({
        "signed": signed_summary(&signed_headers)?,
        "body": signed.body.map(|bytes| STANDARD.encode(bytes)),
        "error": error_value(case.error.as_ref())?,
    }))
}

fn option_case(warnings: &Warnings, case: &OptionCase) -> Case {
    drop(warnings.take());
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
        "warnings": warnings.take(),
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
        // ImageFetchError is LiteLLM's BadRequestError, so the caller answers 400
        Err(ResponsesError::Fetch(_)) => Ok(json!({"error": {"status": 400}, "fetched": fetched})),
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

fn codex_case(warnings: &Warnings, case: &CodexCase) -> Case {
    drop(warnings.take());
    let body =
        transform_inlined_request(&case.model, &case.input, &case.params, None).or_value_error()?;
    Ok(json!({
        "body": serde_json::to_value(body).or_value_error()?,
        "warnings": warnings.take(),
    }))
}
