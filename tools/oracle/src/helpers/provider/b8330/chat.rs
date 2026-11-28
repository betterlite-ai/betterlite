use std::sync::{Mutex, PoisonError};

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::compatible_wire::header_map;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_provider::data_cloud_dispatch::call::CompletionCall;
use betterlite_provider::data_cloud_dispatch::chat::dispatch;
use betterlite_provider::data_cloud_dispatch::context::{
    DispatchContext, DispatchGlobals, EnvironmentSource, oci_explicit_params,
};
use betterlite_provider::data_cloud_dispatch::embed::databricks_environment;
use betterlite_provider::data_cloud_dispatch::error::DispatchError;
use betterlite_provider::data_cloud_dispatch::handler::{ChatCompletionHandler, HandlerError};
use betterlite_provider::data_cloud_dispatch::oauth::OauthM2m;
use betterlite_provider::databricks_auth::oauth::OauthTokenSource;
use betterlite_provider::databricks_chat::request::{
    EnvironmentRequest, get_complete_url as databricks_url,
    validate_environment as databricks_headers,
};
use betterlite_provider::oci_chat::config::{
    get_complete_url as oci_url, validate_environment as oci_headers,
};
use betterlite_provider::oci_common::credentials::{OciCredentials, resolve_oci_credentials};
use betterlite_provider::oci_common::environment::OciEnvironment;
use betterlite_provider::snowflake::config::{
    Endpoint as SnowflakeEndpoint, get_complete_url as snowflake_url,
    validate_environment as snowflake_headers,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::HeaderMap;
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use tokio::runtime::Runtime;

use super::fixtures::{ORACLE_UPSTREAM, Scripted, environment, logging, secrets};
use super::input::{Case, Provider};
use super::wire::{error_or_null, request_view};

const ORACLE_TOKEN: &str = "oracle-token";

// the case values the dispatch reads, owned so the context can borrow them
struct CallSetup {
    optional_params: Object,
    litellm_params: Object,
    api_key: Option<SecretString>,
    api_base: Option<String>,
}

impl CallSetup {
    fn new(case: &Case) -> Result<Self, Box<HelperError>> {
        Ok(Self {
            optional_params: case.optional_params()?,
            litellm_params: Object::new(),
            api_key: case
                .api_key
                .as_ref()
                .map(|value| SecretString::from(value.clone())),
            api_base: case
                .api_base
                .as_deref()
                .map(|value| value.replace("{{upstream}}", ORACLE_UPSTREAM)),
        })
    }

    fn context<'setup>(
        &'setup self,
        case: &'setup Case,
        source: EnvironmentSource<'setup>,
    ) -> DispatchContext<'setup> {
        DispatchContext {
            acompletion: false,
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            custom_llm_provider: case.provider.slug(),
            headers: case.headers.as_ref(),
            litellm_params: &self.litellm_params,
            messages: &case.messages,
            model: &case.model,
            optional_params: &self.optional_params,
            provider_config: None,
            stream: case.stream,
            timeout: None,
            globals: DispatchGlobals::default(),
            env: source,
        }
    }
}

struct Recorder {
    calls: Mutex<Vec<CompletionCall>>,
    failure: Option<u16>,
}

impl Recorder {
    fn first(&self) -> Result<CompletionCall, Box<HelperError>> {
        let recorded = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        recorded.first().map_or_else(
            || {
                Err(super::input_failure(String::from(
                    "the chat arm called no handler",
                )))
            },
            |call| Ok(call.clone()),
        )
    }
}

impl ChatCompletionHandler for Recorder {
    type Reply = String;

    fn complete(&self, call: &CompletionCall) -> Result<Self::Reply, HandlerError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        self.failure.map_or_else(
            || Ok(String::from("reply")),
            |status_code| Err(HandlerError::Upstream { status_code }),
        )
    }
}

fn recorder(case: &Case) -> Recorder {
    Recorder {
        calls: Mutex::new(Vec::new()),
        // litellm's handler succeeds on a 2xx answer and raises on any other status
        failure: case
            .answer
            .as_ref()
            .map(|answer| answer.status)
            .filter(|status| !(200..300).contains(status)),
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

fn dispatched(
    tag: ChatConfigTag,
    context: &DispatchContext<'_>,
    handler: &Recorder,
    log: &mut LogContext<'_, '_>,
) -> Result<(Result<String, DispatchError>, CompletionCall), Box<HelperError>> {
    let outcome = dispatch(tag, context, handler, log)
        .ok_or_else(|| super::input_failure(String::from("the chat table has no arm")))?;
    let call = handler.first()?;
    Ok((outcome, call))
}

pub(super) fn run(runtime: &Runtime, case: &Case) -> HelperResult {
    let env = environment(case);
    let secrets = secrets();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let (mut state, prepost) = logging(&redactor)?;
    let source = EnvironmentSource {
        secrets: &secrets,
        environment: &env,
    };
    let setup = CallSetup::new(case)?;
    let token_transport = Scripted::with_replies(vec![token_reply(ORACLE_TOKEN)]);
    let tokens = OauthM2m {
        transport: &token_transport,
    };
    let handler = recorder(case);
    let context = setup.context(case, source);
    let mut log = LogContext {
        state: &mut state,
        env: &prepost,
    };
    let (outcome, call) = dispatched(case.provider.chat_tag(), &context, &handler, &mut log)?;
    let (url, headers) = runtime.block_on(request_for(
        case,
        &call,
        source,
        &tokens,
        &setup.optional_params,
    ))?;
    let request = request_view(&url, &headers, case.provider.kept_headers(), None);
    Ok(json!({"request": request, "error": error_or_null(&outcome)}))
}

async fn request_for(
    case: &Case,
    call: &CompletionCall,
    source: EnvironmentSource<'_>,
    tokens: &dyn OauthTokenSource,
    optional_params: &Object,
) -> Result<(String, HeaderMap), Box<HelperError>> {
    match case.provider {
        Provider::Databricks => databricks_request(call, source, tokens, optional_params).await,
        Provider::Snowflake => snowflake_request(call, source, optional_params),
        Provider::Oci => oci_request(call, &case.messages, source, optional_params),
    }
}

async fn databricks_request(
    call: &CompletionCall,
    source: EnvironmentSource<'_>,
    tokens: &dyn OauthTokenSource,
    optional_params: &Object,
) -> Result<(String, HeaderMap), Box<HelperError>> {
    let databricks = databricks_environment(&source).or_value_error()?;
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
        &databricks,
        tokens,
    )
    .await
    .or_value_error()?;
    let url = databricks_url(call.api_base.as_deref(), &call.model).or_value_error()?;
    Ok((url, header_map(&resolved).or_value_error()?))
}

fn snowflake_request(
    call: &CompletionCall,
    source: EnvironmentSource<'_>,
    optional_params: &Object,
) -> Result<(String, HeaderMap), Box<HelperError>> {
    let mut params = optional_params.clone();
    let account_id = source
        .environment
        .get("SNOWFLAKE_ACCOUNT_ID")
        .map(String::as_str);
    let endpoint = SnowflakeEndpoint::Chat(&call.model);
    let url = snowflake_url(call.api_base.as_deref(), &endpoint, &mut params, account_id)
        .or_value_error()?;
    let key = call.api_key.as_ref().map(ExposeSecret::expose_secret);
    let mut headers = call.headers.clone().unwrap_or_default();
    snowflake_headers(&mut headers, &call.model, key).or_value_error()?;
    Ok((url, header_map(&headers).or_value_error()?))
}

fn oci_request(
    call: &CompletionCall,
    messages: &[Message],
    source: EnvironmentSource<'_>,
    optional_params: &Object,
) -> Result<(String, HeaderMap), Box<HelperError>> {
    let (credentials, headers) = oci_prepared(call, messages, source, optional_params)?;
    oci_url(&credentials, call.api_base.as_deref())
        .or_value_error()
        .map(|url| (url, headers))
}

// the OCI chat config signs the converted messages, so the credentials and headers come out together
fn oci_prepared(
    call: &CompletionCall,
    messages: &[Message],
    source: EnvironmentSource<'_>,
    optional_params: &Object,
) -> Result<(OciCredentials, HeaderMap), Box<HelperError>> {
    let mut headers = call.headers.as_ref().map_or_else(
        || Ok(HeaderMap::new()),
        |given| header_map(given).or_value_error(),
    )?;
    let explicit = oci_explicit_params(optional_params).or_value_error()?;
    let oci_environment = source.oci().or_value_error()?;
    let credentials = resolve_oci_credentials(&explicit, &oci_environment);
    oci_headers(
        &mut headers,
        &chat_messages(messages)?,
        &credentials,
        &OciEnvironment::default(),
    )
    .or_value_error()?;
    Ok((credentials, headers))
}
