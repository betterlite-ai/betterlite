use std::collections::VecDeque;
use std::mem::take;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::bfl::generation::BflImageResponse;
use betterlite_core::api::bfl::generation_call::error::GenerationCallError;
use betterlite_core::api::bfl::generation_call::params::CallParams;
use betterlite_core::api::bfl::generation_call::{
    GenerationCall, GenerationExecutor, LogContext, prepare_call,
};
use betterlite_core::api::vector_store_http_write::service::SuppliedClient;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::effects::{Effects, Script, Timeline};
use super::failure;
use super::input::Job;
use super::logging::{QuietSink, logging_env, logging_state, pre_call_effect};
use betterlite_oracle_helper_api::helper::HelperResult;

pub(super) fn run_job(job: &Job, runtime: &Runtime) -> HelperResult {
    let effects = Effects::default();
    let (redactor, mut state) = logging_state()?;
    let sink = QuietSink;
    let params = job.params().map_err(|error| failure(&error))?;
    let extra_headers = job.extra_headers();
    let configured_key = job.configured.api_key.as_deref().map(SecretString::from);
    let call = generation_call(job, &params, &extra_headers, configured_key.as_ref());
    let timeline = Timeline::new(&job.clock, &effects);
    let script = Script {
        submission: job.submission.clone(),
        polls: Mutex::new(VecDeque::from(job.polls.clone())),
        effects: &effects,
    };
    let executor = GenerationExecutor {
        provider: &script,
        clock: &timeline,
        sleeper: &timeline,
    };
    let supplied: Option<SuppliedClient<'_, Script<'_>>> = None;
    let logger = |details: &Json| -> Result<(), PrePostError> {
        effects.record(pre_call_effect(details));
        Ok(())
    };
    let env = logging_env(&redactor, &sink, &logger)?;
    let prepared = prepare_call(
        &call,
        &mut LogContext {
            state: &mut state,
            env: &env,
        },
    )
    .map_err(|error| failure(&error))?;
    let result =
        runtime.block_on(executor.image_generation(&call, &prepared, supplied, Vec::new()));
    let recorded = take(&mut *effects.0.lock().unwrap_or_else(PoisonError::into_inner));
    Ok(json!({"effects": recorded, "outcome": outcome(result)}))
}

fn generation_call<'borrow>(
    job: &'borrow Job,
    params: &'borrow CallParams,
    extra_headers: &'borrow [(String, String)],
    configured_key: Option<&'borrow SecretString>,
) -> GenerationCall<'borrow> {
    GenerationCall {
        model: &job.model,
        prompt: &job.prompt,
        optional_params: &job.optional_params,
        params: params.clone(),
        extra_headers,
        timeout: job.timeout_millis.map(Duration::from_millis),
        aimg_generation: false,
        configured_key,
        configured_base: &job.configured.api_base,
        now_seconds: 0,
    }
}

fn outcome(result: Result<BflImageResponse, GenerationCallError>) -> Value {
    match result {
        Ok(response) => {
            let urls: Vec<Option<String>> = response
                .data
                .iter()
                .map(|datum| datum.url.clone())
                .collect();
            json!({"ok": true, "urls": urls})
        }
        Err(error) => json!({"ok": false, "status": error.status()}),
    }
}
