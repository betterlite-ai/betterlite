use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::local_servers::ProviderEnvironment;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::error::WatsonxError;
use betterlite_openai::watsonx_auth::transport::WatsonxTransport;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use betterlite_openai::watsonx_chat::error::WatsonxChatError;
use betterlite_openai::watsonx_chat::params::{SUPPORTED_PARAMS, map_openai_params};
use betterlite_openai::watsonx_chat::plan::{WatsonxChatPlan, WatsonxChatRequest};
use betterlite_openai::watsonx_chat::provider::provider_info;
use betterlite_openai::watsonx_chat::urls::get_complete_url;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    env: BTreeMap<String, String>,
    maps: Vec<MapCase>,
    infos: Vec<InfoCase>,
    urls: Vec<UrlCase>,
    plans: Vec<PlanCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapCase {
    non_default: Object,
    optional: Object,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InfoCase {
    api_base: Option<String>,
    api_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlCase {
    api_base: Option<String>,
    model: String,
    stream: bool,
    options: WatsonxParams,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanCase {
    model: String,
    api_base: Option<String>,
    api_key: Option<String>,
    headers: BTreeMap<String, String>,
    options: WatsonxParams,
}

// the plan never reaches the IAM endpoint in a case, so a send is the failure the case must not hit
struct ClosedTransport;

impl WatsonxTransport for ClosedTransport {
    fn send_buffered(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Err(TransportError::Closed))
    }
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn watsonx_environment(values: &BTreeMap<String, String>) -> WatsonxEnvironment {
    let secret = |name: &str| {
        values
            .get(name)
            .map(|value| SecretString::from(value.clone()))
    };
    let text = |name: &str| values.get(name).cloned();
    WatsonxEnvironment {
        watsonx_iam_url: text("WATSONX_IAM_URL"),
        wx_api_key: secret("WX_API_KEY"),
        watsonx_api_key: secret("WATSONX_API_KEY"),
        watsonx_apikey: secret("WATSONX_APIKEY"),
        watsonx_zenapikey: secret("WATSONX_ZENAPIKEY"),
        watsonx_token: secret("WATSONX_TOKEN"),
        watsonx_api_base: text("WATSONX_API_BASE"),
        watsonx_url: text("WATSONX_URL"),
        wx_url: text("WX_URL"),
        wml_url: text("WML_URL"),
        watsonx_project_id: text("WATSONX_PROJECT_ID"),
        wx_project_id: text("WX_PROJECT_ID"),
        project_id: text("PROJECT_ID"),
        watsonx_region: text("WATSONX_REGION"),
        wx_region: text("WX_REGION"),
        region: text("REGION"),
        watsonx_deployment_space_id: text("WATSONX_DEPLOYMENT_SPACE_ID"),
        watsonx_space_id: text("WATSONX_SPACE_ID"),
        wx_space_id: text("WX_SPACE_ID"),
        space_id: text("SPACE_ID"),
    }
}

// LiteLLM raises WatsonXAIError with the status, and the runner compares the status of a failed call
fn error_failure(error: &WatsonxChatError) -> Box<HelperError> {
    let status = if let WatsonxChatError::Auth(inner) = error
        && let WatsonxError::Api { status_code, .. } = inner.as_ref()
    {
        Some(*status_code)
    } else {
        None
    };
    Box::new(HelperError {
        class: "WatsonxChatError".to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn map_case(case: &MapCase) -> Result<Value, Box<HelperError>> {
    let mut optional_params = case.optional.clone();
    map_openai_params(&case.non_default, &mut optional_params)
        .map_err(|source| failure("MapError", source.to_string()))?;
    Ok(json!({"optional": optional_params}))
}

fn info_case(case: &InfoCase, environment: &ProviderEnvironment) -> Value {
    let key = case.api_key.as_deref().map(SecretString::from);
    let found = provider_info(case.api_base.as_deref(), key.as_ref(), environment);
    json!({"api_base": found.api_base, "api_key": found.api_key.expose_secret()})
}

fn url_case(case: &UrlCase, environment: &WatsonxEnvironment) -> Result<Value, Box<HelperError>> {
    let mut options = case.options.clone();
    let url = get_complete_url(
        case.api_base.as_deref(),
        &case.model,
        case.stream,
        &mut options,
        environment,
    )
    .map_err(|error| error_failure(&error))?;
    Ok(json!({"url": url, "options": options}))
}

fn header_map(headers: &BTreeMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut found = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::try_from(name.as_str())
            .map_err(|source| failure("InvalidHeaderName", source.to_string()))?;
        let header_value = HeaderValue::try_from(value.as_str())
            .map_err(|source| failure("InvalidHeaderValue", source.to_string()))?;
        found.insert(header_name, header_value);
    }
    Ok(found)
}

async fn plan_case(
    case: &PlanCase,
    auth: &WatsonxAuth<ClosedTransport>,
) -> Result<Value, Box<HelperError>> {
    let key = case.api_key.as_deref().map(SecretString::from);
    let request = WatsonxChatRequest {
        model: &case.model,
        api_base: case.api_base.as_deref(),
        api_key: key.as_ref(),
        headers: header_map(&case.headers)?,
        options: case.options.clone(),
    };
    let plan = WatsonxChatPlan::prepare(auth, request)
        .await
        .map_err(|error| error_failure(&error))?;
    let headers: BTreeMap<String, String> = plan
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    // OpenAILikeChatHandler.completion writes stream as a boolean before it builds the body
    let mut options = plan.options;
    let streaming = options.get("stream") == Some(&Value::Bool(true));
    options.insert("stream".to_owned(), Value::Bool(streaming));
    Ok(json!({"plan": {
        "model": plan.model,
        "url": plan.url,
        "headers": headers,
        "options": options,
    }}))
}

fn vllm_environment(values: &BTreeMap<String, String>) -> ProviderEnvironment {
    ProviderEnvironment {
        api_base: values.get("HOSTED_VLLM_API_BASE").cloned(),
        api_key: values
            .get("HOSTED_VLLM_API_KEY")
            .map(|value| SecretString::from(value.clone())),
    }
}

async fn execute(input: &Input) -> HelperResult {
    let environment = watsonx_environment(&input.env);
    let provider_environment = vllm_environment(&input.env);
    let auth = WatsonxAuth::new(
        watsonx_environment(&input.env),
        ClosedTransport,
        Arc::new(|| 0.0_f64),
    )
    .map_err(|source| failure("AuthSetup", source.to_string()))?;
    let maps = input
        .maps
        .iter()
        .map(map_case)
        .collect::<Result<Vec<_>, _>>()?;
    let infos: Vec<Value> = input
        .infos
        .iter()
        .map(|case| info_case(case, &provider_environment))
        .collect();
    let urls = input
        .urls
        .iter()
        .map(|case| url_case(case, &environment))
        .collect::<Result<Vec<_>, _>>()?;
    let mut plans = Vec::new();
    for case in &input.plans {
        plans.push(plan_case(case, &auth).await?);
    }
    Ok(json!({
        "supported": SUPPORTED_PARAMS,
        "maps": maps,
        "infos": infos,
        "urls": urls,
        "plans": plans,
    }))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|source| failure("RuntimeError", source.to_string()))?;
        runtime.block_on(execute(input))
    }
}
