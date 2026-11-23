use super::fixtures::{
    Configs, Fixture, NoInterception, Nothing, Outbound, ScriptedResolver, scripted,
};
use super::ports::Bridge;
use super::{Call, Outcome, RunOutcome};
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::Failure;
use betterlite_core::api::response_id_traversal::response_id::transcode;
use betterlite_core::api::responses_create::bridge::Created;
use betterlite_core::api::responses_create::call::ResponsesCreateCall;
use betterlite_core::api::responses_create::deps::ResponsesCreateDeps;
use betterlite_core::api::responses_create::error::CreateError;
use betterlite_core::api::responses_create::execute::create;
use betterlite_core::api::responses_create_http::service::{CreateHooks, LogContext};
use betterlite_core::api::responses_hook_policy::callbacks::CallbackLists;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::call_helpers::drop_params::drop_params_env_flag;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use secrecy::SecretString;
use std::collections::BTreeMap;
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
    let bridge = Bridge {
        api_base: api_base.to_owned(),
        env: env.clone(),
    };
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
    run_all(call, api_base, &deps).await
}
async fn run_all(
    call: &Call,
    api_base: &str,
    deps: &ResponsesCreateDeps<'_, Outbound, Configs, Nothing, Nothing, ()>,
) -> Result<Outcome, SdkHttpError> {
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        runs.push(run_once(run, api_base, deps).await?);
    }
    Ok(Outcome { runs })
}
async fn run_once(
    run: &Json,
    api_base: &str,
    deps: &ResponsesCreateDeps<'_, Outbound, Configs, Nothing, Nothing, ()>,
) -> Result<RunOutcome, SdkHttpError> {
    let request = create_call(run, api_base)?;
    let mut fixture = Fixture::new("responses")?;
    let env = PrePostEnv {
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
        env: &env,
    };
    let answer = create(deps, &request, (), &mut log).await;
    match answer {
        Ok(created) => Ok(RunOutcome {
            result: Some(created_json(created)?),
            failure: None,
        }),
        Err(error) => Ok(failure_outcome(&error, &request, &fixture.redactor)),
    }
}
fn env_flags(env: &BTreeMap<String, String>) -> (bool, bool) {
    let drop_params = drop_params_env_flag(env.get("LITELLM_DROP_PARAMS").map(String::as_str));
    let auto_summary = env
        .get("LITELLM_REASONING_AUTO_SUMMARY")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    (drop_params.enabled, auto_summary)
}
fn failure_outcome(
    error: &CreateError,
    request: &ResponsesCreateCall,
    redactor: &Redactor,
) -> RunOutcome {
    let (provider, model) = scripted(&request.model, request.custom_llm_provider.as_deref());
    let failure = error.sdk_failure(&provider, &model, redactor);
    RunOutcome {
        result: None,
        failure: Some(Failure {
            class: failure.class.to_owned(),
            status: failure.status,
            kind: Some(failure.kind),
            param: Some(failure.param),
            code: Some(failure.code),
            message: failure.message,
        }),
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
            "a chat outcome is not handled here",
        ))),
        Created::Live(_) => Err(SdkHttpError::Invalid(String::from(
            "a live stream is not a create answer",
        ))),
        Created::Synthetic(_) => Err(SdkHttpError::Invalid(String::from(
            "a synthetic stream is not a create answer",
        ))),
    }
}
