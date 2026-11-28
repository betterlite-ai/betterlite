use std::collections::BTreeMap;
use std::fmt::Display;
use std::future::{Future, ready};
use std::time::{Duration, UNIX_EPOCH};

use aws_smithy_async::time::{SharedTimeSource, StaticTimeSource};
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::sagemaker_runtime::chat::{self, ChatCall, ChatVariant};
use betterlite_bedrock::sagemaker_runtime::endpoint::{EndpointRequest, ParamMap, runtime_url};
use betterlite_bedrock::sagemaker_runtime::runtime::{
    PreparedCall, SagemakerEnvironment, SagemakerRuntime,
};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::compatible_wire::{NEW_ID, ReplyOptions, system_clock};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::sagemaker_chat::request;
use betterlite_openai::sagemaker_completion::response::TextTokenCounter;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::value::to_raw_value;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use uuid::Uuid;

const REGION_FIELD: &str = "aws_region_name";

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Url(case) => Ok(url_result(case)),
            Input::ComponentHeader(case) => Ok(component_result(case)),
            Input::SignedRequest(case) => signed_result(case),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Url(UrlInput),
    ComponentHeader(ComponentHeaderInput),
    SignedRequest(SignedRequestInput),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlInput {
    model: String,
    region: String,
    base_url: Option<String>,
    stream: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ComponentHeaderInput {
    model_id: Option<String>,
    headers: Headers,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignedRequestInput {
    model: String,
    region: String,
    headers: Headers,
    optional_params: Object,
    messages: Vec<ChatMessage>,
    credentials: CredentialInput,
    bearer: Option<SecretString>,
    now: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialInput {
    access_key_id: SecretString,
    secret_access_key: SecretString,
    session_token: Option<SecretString>,
}

fn url_result(case: &UrlInput) -> Value {
    json!({
        "url": runtime_url(&EndpointRequest {
            model: &case.model,
            region: &case.region,
            base_url: case.base_url.as_deref(),
            stream: case.stream,
        }),
    })
}

fn component_result(case: &ComponentHeaderInput) -> Value {
    let mut optional_params = Object::new();
    if let Some(component) = case.model_id.as_ref() {
        optional_params.insert(
            String::from("model_id"),
            PromptJson::Text(component.clone()),
        );
    }
    json!({"headers": request::validate_environment(case.headers.clone(), &optional_params)})
}

struct FixedCredentials(Credentials);

impl CredentialSource for FixedCredentials {
    fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        ready(Ok(self.0.clone()))
    }
}

struct ZeroCounter;

impl TextTokenCounter for ZeroCounter {
    fn count_response_tokens(&self, _text: &str) -> u64 {
        0
    }
}

fn signed_result(case: &SignedRequestInput) -> HelperResult {
    let credentials = Credentials {
        access_key_id: case.credentials.access_key_id.clone(),
        secret_access_key: case.credentials.secret_access_key.clone(),
        session_token: case.credentials.session_token.clone(),
    };
    let stamp = UNIX_EPOCH
        .checked_add(Duration::from_secs(case.now))
        .unwrap_or(UNIX_EPOCH);
    let mut signing = ParamMap::new();
    signing.insert(
        String::from(REGION_FIELD),
        to_raw_value(&PromptJson::Text(case.region.clone())).or_value_error()?,
    );
    let counter = ZeroCounter;
    let sagemaker = SagemakerRuntime {
        transport: (),
        credentials: FixedCredentials(credentials),
        environment: SagemakerEnvironment::default(),
        clock: SharedTimeSource::new(StaticTimeSource::new(stamp)),
        counter: &counter,
        timeout: Duration::from_secs(5),
        identifier: Uuid::nil,
    };
    let prepared = prepare_call(&sagemaker, case, &signing)?;
    let headers: BTreeMap<String, String> = prepared
        .headers
        .into_iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value))
        .collect();
    Ok(json!({
        "ok": true,
        "method": "POST",
        "url": prepared.url,
        "headers": headers,
        "body": serde_json::from_slice::<Value>(&prepared.body).map_err(failure)?,
    }))
}

fn prepare_call(
    sagemaker: &SagemakerRuntime<'_, (), FixedCredentials>,
    case: &SignedRequestInput,
    signing: &ParamMap,
) -> Result<PreparedCall, Box<HelperError>> {
    let call = ChatCall {
        model: &case.model,
        messages: &case.messages,
        optional_params: &case.optional_params,
        signing,
        headers: case.headers.clone(),
        hf_model_name: None,
        variant: ChatVariant::Chat,
        stream: false,
        reply: ReplyOptions {
            json_mode: false,
            provider: None,
            base_model: None,
            identifier: NEW_ID,
            clock: system_clock,
        },
        configured_bearer: case.bearer.as_ref(),
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)?;
    runtime
        .block_on(chat::prepare_chat(sagemaker, &call))
        .map_err(failure)
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "SagemakerRuntimeError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
