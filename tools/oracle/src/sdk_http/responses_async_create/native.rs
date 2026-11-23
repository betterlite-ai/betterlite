use super::fixtures::{
    Configs, Fixture, NoBridge, NoInterception, Nothing, Outbound, ScriptedResolver,
};
use super::scripted::{HookScript, ScriptedPort};
use super::{Call, Outcome, RunOutcome};
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::Failure;
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
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::call_helpers::drop_params::drop_params_env_flag;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use secrecy::SecretString;
use std::collections::BTreeMap;

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
    let deps = ResponsesCreateDeps {
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
    let hook = hook_script(run)?;
    let enabled = hook.is_some();
    let scripted = ScriptedPort::new(hook);
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
    let answer = wrapped(deps, &request, enabled.then_some(&scripted), &mut log).await;
    let hooks = scripted.observation();
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

// The wrapper aresponses runs: its own provider detection, its optional async prompt port and the sync execution behind it
async fn wrapped(
    deps: &CreateDeps<'_>,
    request: &ResponsesCreateCall,
    prompts: Option<&dyn AsyncPromptPort>,
    log: &mut LogContext<'_, '_>,
) -> Result<Created<()>, AsyncCreateError> {
    let execution = CreateExecution { deps };
    let create = AsyncCreateDeps {
        resolver: deps.resolver,
        lookup: deps.lookup,
        prompts,
    };
    aresponses(&create, &execution, request, (), log).await
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
