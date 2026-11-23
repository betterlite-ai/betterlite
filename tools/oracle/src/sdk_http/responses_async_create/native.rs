use super::fixtures::{
    Configs, Fixture, NoBridge, NoInterception, Nothing, Outbound, ScriptedResolver,
};
use super::{Call, HookCall, HookObservation, Outcome, RunOutcome};
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::Failure;
use async_trait::async_trait;
use betterlite_core::api::response_id_traversal::response_id::transcode;
use betterlite_core::api::responses_create::bridge::Created;
use betterlite_core::api::responses_create::call::ResponsesCreateCall;
use betterlite_core::api::responses_create::deps::ResponsesCreateDeps;
use betterlite_core::api::responses_create_call::aresponses;
use betterlite_core::api::responses_create_call::deps::{
    AsyncCreateDeps, AsyncPromptPort, CreateExecution,
};
use betterlite_core::api::responses_create_call::error::AsyncCreateError;
use betterlite_core::api::responses_create_http::service::{CreateHooks, LogContext};
use betterlite_core::api::responses_hook_policy::callbacks::CallbackLists;
use betterlite_core::api::responses_json::{decision_json, json_value};
use betterlite_core::api::responses_parameters::prompt_merge::ReturnedMessage;
use betterlite_core::api::responses_provider_selection::ParameterMap;
use betterlite_core::api::responses_provider_selection::prompting::{
    CARRY_UNMATCHED_MESSAGE_POINTS, HookRequest, MergeAnswer, MergeRequest, PromptManagerError,
};
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::call_helpers::drop_params::drop_params_env_flag;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use secrecy::SecretString;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};

type CreateDeps<'deps> = ResponsesCreateDeps<'deps, Outbound, Configs, Nothing, Nothing, ()>;

pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<Outcome, SdkHttpError> {
    let origin = Outbound::new(env)?;
    let resolver = ScriptedResolver;
    let configs = Configs {
        api_key: env.get("OPENAI_API_KEY").cloned().map(SecretString::from),
    };
    let interception = NoInterception;
    let bridge = NoBridge;
    let nothing = Nothing;
    let (drop_params, auto_summary) = env_flags(env);
    let deps = CreateDeps {
        resolver: &resolver,
        lookup: &configs,
        configs: &configs,
        source: &origin,
        supplied: None,
        environment: env,
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
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        runs.push(run_once(run, api_base, &deps).await?);
    }
    Ok(Outcome { runs })
}

fn env_flags(env: &BTreeMap<String, String>) -> (bool, bool) {
    let drop_params = drop_params_env_flag(env.get("LITELLM_DROP_PARAMS").map(String::as_str));
    let auto_summary = env
        .get("LITELLM_REASONING_AUTO_SUMMARY")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    (drop_params.enabled, auto_summary)
}

async fn run_once(
    run: &Json,
    api_base: &str,
    deps: &CreateDeps<'_>,
) -> Result<RunOutcome, SdkHttpError> {
    let request = create_call(run, api_base)?;
    let port = hook_script(run)?.map(ScriptedPort::new);
    let mut fixture = Fixture::new("responses")?;
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
        state: &mut fixture.state,
        env: &environment,
    };
    let execution = CreateExecution { deps };
    let prompts = port.as_ref().map(|port| port as &dyn AsyncPromptPort);
    let create = AsyncCreateDeps {
        resolver: deps.resolver,
        lookup: deps.lookup,
        prompts,
    };
    let answer = aresponses(&create, &execution, &request, (), &mut log).await;
    let hooks = match &port {
        Some(scripted) => scripted.observation(),
        None => HookObservation::default(),
    };
    match answer {
        Ok(created) => Ok(RunOutcome {
            result: Some(created_json(created)?),
            failure: None,
            hooks,
        }),
        Err(error) => Ok(RunOutcome {
            result: None,
            failure: Some(failure_outcome(&error, &fixture.redactor)),
            hooks,
        }),
    }
}

fn failure_outcome(error: &AsyncCreateError, redactor: &Redactor) -> Failure {
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

fn create_call(run: &Json, api_base: &str) -> Result<ResponsesCreateCall, SdkHttpError> {
    let mut call: ResponsesCreateCall =
        transcode(run).map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    call.api_base = Some(api_base.to_owned());
    if let Json::Object(fields) = run {
        call.timeout = match fields.get("timeout_seconds") {
            Some(Json::Number(seconds)) => seconds.as_f64(),
            None
            | Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
                call.timeout
            }
        };
    }
    Ok(call)
}

fn created_json(created: Created<()>) -> Result<Json, SdkHttpError> {
    match created {
        Created::Responses(stamped) => {
            transcode(&stamped.response).map_err(|error| SdkHttpError::Invalid(error.to_string()))
        }
        Created::Raw(raw) => serde_json::from_str(raw.get())
            .map_err(|error| SdkHttpError::Invalid(error.to_string())),
        Created::Chat(_) => Err(SdkHttpError::Invalid(String::from(
            "a chat outcome is not an async create answer",
        ))),
        Created::Live(_) => Err(SdkHttpError::Invalid(String::from(
            "a live stream is not an async create answer",
        ))),
        Created::Synthetic(_) => Err(SdkHttpError::Invalid(String::from(
            "a synthetic stream is not an async create answer",
        ))),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HookScript {
    answer: HookAnswer,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HookAnswer {
    model: String,
    messages: Vec<Json>,
    #[serde(default)]
    optional_params: Json,
}

fn hook_script(run: &Json) -> Result<Option<HookScript>, SdkHttpError> {
    let Json::Object(fields) = run else {
        return Ok(None);
    };
    match fields.get("hook") {
        None | Some(Json::Null) => Ok(None),
        Some(hook) => transcode(hook)
            .map(Some)
            .map_err(|error| SdkHttpError::Invalid(error.to_string())),
    }
}

// The scripted stand-in for LiteLLMLoggingObj's async prompt hook, counted the way the adapter observes it
struct ScriptedPort {
    answer: HookAnswer,
    checks: AtomicU32,
    ran: AtomicU32,
    records: Mutex<Vec<HookCall>>,
}

impl ScriptedPort {
    fn new(script: HookScript) -> Self {
        Self {
            answer: script.answer,
            checks: AtomicU32::new(0),
            ran: AtomicU32::new(0),
            records: Mutex::new(Vec::new()),
        }
    }

    fn observation(&self) -> HookObservation {
        HookObservation {
            checks: self.checks.load(Ordering::SeqCst),
            ran: self.ran.load(Ordering::SeqCst),
            sync_checks: 0,
            calls: self
                .records
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
    }
}

#[async_trait]
impl AsyncPromptPort for ScriptedPort {
    fn runs_prompt_hooks(&self, request: &HookRequest<'_>) -> bool {
        let runs = prompt_management_runs(request);
        self.checks.fetch_add(1, Ordering::SeqCst);
        if runs {
            self.ran.fetch_add(1, Ordering::SeqCst);
        }
        runs
    }

    async fn async_get_chat_completion_prompt(
        &self,
        request: &MergeRequest<'_>,
    ) -> Result<MergeAnswer, PromptManagerError> {
        let record = HookCall {
            model: request.model.to_owned(),
            prompt_id: request.prompt_id.map(str::to_owned),
            prompt_label: request.prompt_label.map(str::to_owned),
            prompt_version: request.prompt_version.map(decision_json),
            prompt_variables: request.prompt_variables.map(decision_json),
            messages: Json::List(
                request
                    .messages
                    .iter()
                    .map(|message| decision_json(&message.message))
                    .collect(),
            ),
            carry: request
                .request_kwargs
                .contains_key(CARRY_UNMATCHED_MESSAGE_POINTS),
            aresponses: truthy(request.non_default_params.get("aresponses")),
        };
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
        let messages = self
            .answer
            .messages
            .iter()
            .map(|message| ReturnedMessage {
                source: None,
                message: json_value(message),
            })
            .collect();
        let optional_params = match &self.answer.optional_params {
            Json::Null => ParameterMap::new(),
            value => transcode(value).map_err(|error| PromptManagerError {
                message: error.to_string(),
            })?,
        };
        Ok(MergeAnswer {
            model: self.answer.model.clone(),
            messages,
            optional_params,
        })
    }
}

// LiteLLM's Logging.should_run_prompt_management_hooks, for the params aresponses passes it
fn prompt_management_runs(request: &HookRequest<'_>) -> bool {
    if request
        .prompt_id
        .is_some_and(|prompt_id| !prompt_id.is_empty())
    {
        return true;
    }
    [
        "cache_control_injection_points",
        "knowledge_bases",
        "vector_store_ids",
    ]
    .iter()
    .any(|name| truthy(request.non_default_params.get(*name)))
}

fn truthy(value: Option<&JsonValue>) -> bool {
    match value {
        None | Some(JsonValue::Null) => false,
        Some(JsonValue::Bool(flag)) => *flag,
        Some(JsonValue::Number(number)) => number.as_f64().is_some_and(|float| float != 0.0_f64),
        Some(JsonValue::Text(text)) => !text.is_empty(),
        Some(JsonValue::Array(items)) => !items.is_empty(),
        Some(JsonValue::Object(fields)) => !fields.is_empty(),
    }
}
