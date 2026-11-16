use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::chat::ChatMessage;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_provider::data_cloud_dispatch::call::CompletionCall;
use betterlite_provider::data_cloud_dispatch::chat::dispatch;
use betterlite_provider::data_cloud_dispatch::context::{
    CallLogging, DispatchContext, DispatchGlobals, databricks_environment, oci_environment,
    oci_explicit_params,
};
use betterlite_provider::data_cloud_dispatch::handler::{ChatCompletionHandler, HandlerError};
use betterlite_provider::data_cloud_dispatch::oauth::OauthM2m;
use betterlite_provider::databricks_auth::environment::DatabricksEnv;
use betterlite_provider::databricks_auth::oauth::OauthTokenSource;
use betterlite_provider::databricks_chat::request::{
    EnvironmentRequest, get_complete_url as databricks_url,
    validate_environment as databricks_headers,
};
use betterlite_provider::oci_chat::config::{
    get_complete_url as oci_url, validate_environment as oci_headers,
};
use betterlite_provider::oci_common::credentials::resolve_oci_credentials;
use betterlite_provider::oci_common::environment::OciEnvironment;
use betterlite_provider::snowflake::config::{
    Endpoint as SnowflakeEndpoint, get_complete_url as snowflake_url,
    validate_environment as snowflake_headers,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use tokio::runtime::Runtime;

use super::fixture::{ORACLE_UPSTREAM, Scripted, environment, logging, secrets};
use super::input::{Case, Provider};
use super::wire::{error_or_null, header_map, kept_headers};

const ORACLE_TOKEN: &str = "oracle-token";

struct Recorder {
    calls: Mutex<Vec<CompletionCall>>,
    failure: Option<u16>,
}

impl Recorder {
    fn first(&self) -> Result<CompletionCall, Box<HelperError>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .ok_or_else(|| super::input_failure(String::from("the chat arm called no handler")))
    }
}

impl ChatCompletionHandler for Recorder {
    type Reply = String;

    fn complete(&self, call: &CompletionCall) -> Result<Self::Reply, HandlerError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        match self.failure {
            None => Ok(String::from("reply")),
            Some(status_code) => Err(HandlerError::Upstream { status_code }),
        }
    }
}

fn token_reply(token: &str) -> UpstreamResponse {
    UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: format!("{{\"access_token\":\"{token}\"}}").into_bytes(),
    }
}

pub(super) fn chat_messages(messages: &[Message]) -> Result<Vec<ChatMessage>, Box<HelperError>> {
    let mut converted = Vec::with_capacity(messages.len());
    for message in messages {
        converted.push(serde_json::from_value::<ChatMessage>(
            serde_json::to_value(message)?,
        )?);
    }
    Ok(converted)
}

pub(super) fn run(runtime: &Runtime, case: &Case) -> HelperResult {
    let env = environment(case);
    let secrets = secrets();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let (mut state, prepost) = logging(&redactor)?;
    let token_transport = Scripted::with_replies(vec![token_reply(ORACLE_TOKEN)]);
    let tokens = OauthM2m {
        transport: &token_transport,
    };
    let databricks_env = databricks_environment(&secrets, &env).or_value_error()?;
    let optional_params = case.optional_params()?;
    let litellm_params = Object::new();
    let api_key = case
        .api_key
        .as_ref()
        .map(|value| SecretString::from(value.clone()));
    let api_base = case
        .api_base
        .as_deref()
        .map(|value| value.replace("{{upstream}}", ORACLE_UPSTREAM));
    let context = DispatchContext {
        acompletion: false,
        api_base: api_base.as_deref(),
        api_key: api_key.as_ref(),
        custom_llm_provider: case.provider.slug(),
        headers: case.headers.as_ref(),
        litellm_params: &litellm_params,
        messages: &case.messages,
        model: &case.model,
        optional_params: &optional_params,
        provider_config: None,
        stream: case.stream,
        timeout: None,
        globals: DispatchGlobals::default(),
        secrets: &secrets,
        environment: &env,
    };
    let handler = Recorder {
        calls: Mutex::new(Vec::new()),
        // litellm's handler succeeds on a 2xx answer and raises on any other status
        failure: case
            .answer
            .as_ref()
            .map(|answer| answer.status)
            .filter(|status| !(200..300).contains(status)),
    };
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let outcome = dispatch(case.provider.chat_tag(), &context, &handler, &mut log)
        .ok_or_else(|| super::input_failure(String::from("the chat table has no arm")))?;
    let call = handler.first()?;
    let (url, headers) = runtime.block_on(request_for(
        case,
        &call,
        &env,
        &databricks_env,
        &secrets,
        &tokens,
        &optional_params,
    ))?;
    let request = json!({
        "url": url,
        "headers": kept_headers(&headers, case.provider.kept_headers()),
    });
    Ok(json!({
        "request": request,
        "error": error_or_null(&outcome),
    }))
}

async fn request_for(
    case: &Case,
    call: &CompletionCall,
    env: &BTreeMap<String, String>,
    databricks_env: &DatabricksEnv,
    secrets: &SecretLookup,
    tokens: &dyn OauthTokenSource,
    optional_params: &Object,
) -> Result<(String, HeaderMap), Box<HelperError>> {
    match case.provider {
        Provider::Databricks => {
            databricks_request(call, databricks_env, tokens, optional_params).await
        }
        Provider::Snowflake => snowflake_request(call, env, optional_params),
        Provider::Oci => oci_request(case, call, env, secrets, optional_params),
    }
}

async fn databricks_request(
    call: &CompletionCall,
    databricks_env: &DatabricksEnv,
    tokens: &dyn OauthTokenSource,
    optional_params: &Object,
) -> Result<(String, HeaderMap), Box<HelperError>> {
    let mut params = optional_params.clone();
    let resolved = databricks_headers(
        EnvironmentRequest {
            api_key: call.api_key.as_ref(),
            api_base: call.api_base.as_deref(),
            headers: call.headers.clone(),
            litellm_user_agent: None,
            env_user_agent: None,
        },
        &mut params,
        databricks_env,
        tokens,
    )
    .await
    .or_value_error()?;
    let url = databricks_url(call.api_base.as_deref(), &call.model).or_value_error()?;
    Ok((url, header_map(&resolved)))
}

fn snowflake_request(
    call: &CompletionCall,
    env: &BTreeMap<String, String>,
    optional_params: &Object,
) -> Result<(String, HeaderMap), Box<HelperError>> {
    let mut params = optional_params.clone();
    let account_id = env.get("SNOWFLAKE_ACCOUNT_ID").map(String::as_str);
    let url = snowflake_url(
        call.api_base.as_deref(),
        &SnowflakeEndpoint::Chat(&call.model),
        &mut params,
        account_id,
    )
    .or_value_error()?;
    let mut headers = call.headers.clone().unwrap_or_default();
    let key = call
        .api_key
        .as_ref()
        .map(|value| value.expose_secret().clone());
    snowflake_headers(&mut headers, &call.model, key.as_deref()).or_value_error()?;
    Ok((url, header_map(&headers)))
}

fn oci_request(
    case: &Case,
    call: &CompletionCall,
    env: &BTreeMap<String, String>,
    secrets: &SecretLookup,
    optional_params: &Object,
) -> Result<(String, HeaderMap), Box<HelperError>> {
    let explicit = oci_explicit_params(optional_params).or_value_error()?;
    let oci_env = oci_environment(secrets, env).or_value_error()?;
    let credentials = resolve_oci_credentials(&explicit, &oci_env);
    let mut headers = match &call.headers {
        Some(headers) => header_map(headers),
        None => HeaderMap::new(),
    };
    let messages = chat_messages(&case.messages)?;
    oci_headers(
        &mut headers,
        &messages,
        &credentials,
        &OciEnvironment::default(),
    )
    .or_value_error()?;
    let url = oci_url(&credentials, call.api_base.as_deref()).or_value_error()?;
    Ok((url, headers))
}
