use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_provider::api::rerank_contracts::{BaseLlmException, RerankLogging};
use betterlite_provider::api::rerank_http::error::RerankHttpError;
use betterlite_provider::api::rerank_http::{RerankCall, RerankDeps, RerankLogContext, rerank};
use secrecy::SecretString;
use serde::Serialize;

use super::super::error::SdkHttpError;
use super::super::result::Failure;
use super::super::result::kinds::SdkResult;
use super::support::{ConfigError, Fixture, RecordingConfig, Source, source};
use super::{Call, Mode, Outcome, PreCallSeen, RunOutcome, TransformSeen};

// The handler over the mock's base: the same config sections and pooled client a request would build
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let loaded = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let origin = source(&loaded, env)?;
    let key = call.api_key.clone().map(SecretString::from);
    let mut runs = Vec::with_capacity(call.modes.len());
    for mode in &call.modes {
        runs.push(run_once(call, *mode, api_base, &origin, key.as_ref()).await?);
    }
    Ok(SdkResult::RerankHttp {
        value: Box::new(Outcome { runs }),
    })
}

async fn run_once(
    call: &Call,
    mode: Mode,
    api_base: &str,
    origin: &Source,
    key: Option<&SecretString>,
) -> Result<RunOutcome, SdkHttpError> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let config = RecordingConfig::new(Arc::clone(&events));
    let mut fixture = Fixture::new("rerank", Arc::clone(&events))?;
    let mut logging = RerankLogging::default();
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
        logger_fn: Some(&*fixture.logger),
        now: fixture.now,
    };
    let mut log = RerankLogContext {
        state: &mut fixture.state,
        env: &env,
    };
    let deps = RerankDeps {
        config: &config,
        source: origin,
        supplied: None,
    };
    let prepared = rerank_call(call, mode, api_base, key, &mut logging);
    let reply = rerank(prepared, deps, &mut log).await;
    let recorded = events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let pre_call = fixture.pre_call();
    let transform = config.seen();
    outcome(reply, recorded, pre_call, transform)
}

fn rerank_call<'call>(
    call: &'call Call,
    mode: Mode,
    api_base: &'call str,
    key: Option<&'call SecretString>,
    logging: &'call mut RerankLogging,
) -> RerankCall<'call> {
    RerankCall {
        model: &call.model,
        optional_params: &call.optional_params,
        api_base: Some(api_base),
        api_key: key,
        litellm_params: None,
        headers: None,
        timeout: call.timeout_seconds.map(Duration::from_secs),
        is_async: mode.request_flag(),
        model_response: RerankResponse::default(),
        logging,
    }
}

// A provider failure is what both sides can observe; a betterlite-only failure is not a parity answer
fn outcome(
    reply: Result<RerankResponse, RerankHttpError<ConfigError>>,
    events: Vec<String>,
    pre_call: Option<PreCallSeen>,
    transform: Option<TransformSeen>,
) -> Result<RunOutcome, SdkHttpError> {
    match reply {
        Ok(record) => Ok(RunOutcome {
            result: Some(json_of(&record)?),
            failure: None,
            events,
            pre_call,
            transform,
        }),
        Err(RerankHttpError::Provider(error)) => Ok(RunOutcome {
            result: None,
            failure: Some(failure(&error)),
            events,
            pre_call,
            transform,
        }),
        Err(other) => Err(SdkHttpError::Invalid(other.to_string())),
    }
}

fn failure(error: &BaseLlmException) -> Failure {
    Failure {
        class: String::from("BaseLLMException"),
        status: error.status_code,
        kind: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.message.clone(),
    }
}

fn json_of<T: Serialize>(value: &T) -> Result<Json, SdkHttpError> {
    let plain = serde_json::to_value(value).map_err(SdkHttpError::Serialize)?;
    serde_json::from_value(plain).map_err(SdkHttpError::Serialize)
}
