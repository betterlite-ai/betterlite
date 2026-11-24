use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;
use betterlite_core::api::response_chat_dispatch::port::{
    Answer, ResponsesCall, ResponsesExecution,
};
use betterlite_core::api::response_id_traversal::response_id::transcode;
use betterlite_core::api::responses_create::call::ResponsesCreateCall;
use betterlite_core::api::responses_create::deps::ResponsesCreateDeps;
use betterlite_core::api::responses_create::error::CreateError;
use betterlite_core::api::responses_create::execute::create;
use betterlite_core::api::responses_create_call::aresponses;
use betterlite_core::api::responses_create_call::deps::{AsyncCreateDeps, CreateExecution};
use betterlite_core::api::responses_create_call::error::AsyncCreateError;
use betterlite_core::api::responses_create_http::service::{CreateHooks, LogContext};
use betterlite_core::api::responses_hook_policy::callbacks::CallbackLists;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::call_helpers::drop_params::drop_params_env_flag;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Value;

use super::answers::answer;
use super::fixtures::{
    Configs, Fixture, NoBridge, NoInterception, Nothing, Outbound, ScriptedResolver, scripted,
};
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::Failure;

// Both ways responses() can fail behind the bridge, told apart by which entry the bridge took
#[derive(Debug)]
pub(crate) enum ExecutionFailure {
    Sync {
        error: CreateError,
        provider: String,
        model: String,
    },
    Async(AsyncCreateError),
    Sdk(SdkHttpError),
}

impl ExecutionFailure {
    pub(crate) fn failure(&self, redactor: &Redactor) -> Failure {
        match self {
            Self::Sync {
                error,
                provider,
                model,
            } => {
                let mapped = error.sdk_failure(provider, model, redactor);
                Failure {
                    class: mapped.class.to_owned(),
                    status: mapped.status,
                    kind: Some(mapped.kind),
                    param: Some(mapped.param),
                    code: Some(mapped.code),
                    message: mapped.message,
                }
            }
            Self::Async(error) => {
                let mapped = error.sdk_failure(redactor);
                Failure {
                    class: mapped.class.to_owned(),
                    status: mapped.status,
                    kind: Some(mapped.kind),
                    param: Some(mapped.param),
                    code: Some(mapped.code),
                    message: mapped.message,
                }
            }
            Self::Sdk(error) => Failure {
                class: String::from("APIError"),
                status: 500,
                kind: Some(None),
                param: Some(None),
                code: Some(None),
                message: error.to_string(),
            },
        }
    }
}

impl fmt::Display for ExecutionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sync { error, .. } => write!(formatter, "{error}"),
            Self::Async(error) => write!(formatter, "{error}"),
            Self::Sdk(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ExecutionFailure {}

impl From<SdkHttpError> for ExecutionFailure {
    fn from(error: SdkHttpError) -> Self {
        Self::Sdk(error)
    }
}

pub(crate) struct RealExecution<'run> {
    pub(crate) api_base: &'run str,
    pub(crate) env: &'run BTreeMap<String, String>,
}

type CreateDeps<'deps> = ResponsesCreateDeps<'deps, Outbound, Configs, Nothing, Nothing, ()>;

#[async_trait]
impl ResponsesExecution<()> for RealExecution<'_> {
    type Error = ExecutionFailure;

    async fn responses(&self, call: ResponsesCall<'_, ()>) -> Result<Answer, Self::Error> {
        self.run(call, false).await
    }

    async fn aresponses(&self, call: ResponsesCall<'_, ()>) -> Result<Answer, Self::Error> {
        self.run(call, true).await
    }
}

impl RealExecution<'_> {
    async fn run(
        &self,
        call: ResponsesCall<'_, ()>,
        asynchronous: bool,
    ) -> Result<Answer, ExecutionFailure> {
        let ResponsesCall {
            params,
            litellm_params,
            extra_headers,
            logging,
            ..
        } = call;
        let request = create_call(params, litellm_params, extra_headers, self.api_base)?;
        let origin = Outbound::new(self.env)?;
        let resolver = ScriptedResolver;
        let configs = Configs {
            api_key: self
                .env
                .get("OPENAI_API_KEY")
                .cloned()
                .map(SecretString::from),
        };
        let interception = NoInterception;
        let bridge = NoBridge;
        let nothing = Nothing;
        let (drop_params, auto_summary) = env_flags(self.env);
        let deps = ResponsesCreateDeps {
            resolver: &resolver,
            lookup: &configs,
            configs: &configs,
            source: &origin,
            supplied: None,
            environment: self.env,
            prompts: None,
            file_mapping: None,
            hooks: CreateHooks {
                callbacks: CallbackLists {
                    global: &[],
                    dynamic: &[],
                },
                resolver: &nothing,
                dispatcher: &nothing,
                executor: &nothing,
                logging: None,
            },
            interception: &interception,
            bridge: &bridge,
            drop_params,
            reasoning_auto_summary: auto_summary,
        };
        let fixture = Fixture::new("responses")?;
        let environment = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &fixture.redactor,
            log: &fixture.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: fixture.now,
        };
        let mut log = LogContext {
            state: logging,
            env: &environment,
        };
        let created = dispatch_create(&deps, &request, &mut log, asynchronous).await?;
        answer(created)
    }
}

async fn dispatch_create(
    deps: &CreateDeps<'_>,
    request: &ResponsesCreateCall,
    log: &mut LogContext<'_, '_>,
    asynchronous: bool,
) -> Result<Created<()>, ExecutionFailure> {
    if asynchronous {
        let prompts = AsyncCreateDeps {
            resolver: deps.resolver,
            lookup: deps.lookup,
            prompts: None,
        };
        let handoff = CreateExecution { deps };
        aresponses(&prompts, &handoff, request, (), log)
            .await
            .map_err(ExecutionFailure::Async)
    } else {
        let (provider, model) = scripted(&request.model, request.custom_llm_provider.as_deref());
        create(deps, request, (), log)
            .await
            .map_err(|error| ExecutionFailure::Sync {
                error,
                provider,
                model,
            })
    }
}

fn create_call(
    params: &ResponsesApiRequestParams,
    litellm_params: &IndexMap<String, Value>,
    extra_headers: Option<&IndexMap<String, String>>,
    api_base: &str,
) -> Result<ResponsesCreateCall, ExecutionFailure> {
    let mut call: ResponsesCreateCall = transcode(params)
        .map_err(|error| ExecutionFailure::Sdk(SdkHttpError::Invalid(error.to_string())))?;
    call.api_base = Some(api_base.to_owned());
    call.api_key = text(litellm_params.get("api_key")).map(SecretString::from);
    call.custom_llm_provider = text(litellm_params.get("custom_llm_provider"));
    call.timeout = number(litellm_params.get("timeout"));
    call.extra_headers = match extra_headers {
        Some(headers) => Some(
            headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        ),
        None => None,
    };
    Ok(call)
}

fn text(value: Option<&Value>) -> Option<String> {
    let Some(Value::String(text)) = value else {
        return None;
    };
    Some(text.clone())
}

fn number(value: Option<&Value>) -> Option<f64> {
    let Some(Value::Number(number)) = value else {
        return None;
    };
    number.as_f64()
}

pub(crate) fn env_flags(env: &BTreeMap<String, String>) -> (bool, bool) {
    let drop_params = drop_params_env_flag(env.get("LITELLM_DROP_PARAMS").map(String::as_str));
    let auto_summary = env
        .get("LITELLM_REASONING_AUTO_SUMMARY")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    (drop_params.enabled, auto_summary)
}
