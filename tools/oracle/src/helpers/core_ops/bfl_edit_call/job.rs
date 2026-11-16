use std::collections::BTreeMap;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::bfl::edit::media::{MaskInput, SuppliedMedia};
use betterlite_core::api::bfl::edit_call::error::EditCallError;
use betterlite_core::api::bfl::edit_call::{EditCall, EditExecutor, prepare_call};
use betterlite_core::api::bfl::generation::BflImageResponse;
use betterlite_core::api::bfl::generation_call::LogContext;
use betterlite_core::api::bfl::generation_call::submit::SuppliedTransport;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::effects::Wire;
use super::failure;
use super::input::{ImageSpec, Job};
use super::logging::{QuietSink, logging_env, logging_state, pre_call_effect};

pub(super) fn run_job(job: &Job, runtime: &Runtime) -> HelperResult {
    let wire = Wire::new(job);
    let (redactor, mut state) = logging_state()?;
    let sink = QuietSink;
    let logger = |details: &Json| -> Result<(), PrePostError> {
        wire.record(pre_call_effect(details));
        Ok(())
    };
    let env = logging_env(&redactor, &sink, &logger)?;
    let max_depth = max_depth()?;
    let mut image = media_from(&job.image)?;
    let result = runtime.block_on(execute(
        job,
        &wire,
        &mut LogContext {
            state: &mut state,
            env: &env,
        },
        &mut image,
        max_depth,
    ));
    let recorded = wire.take_effects();
    Ok(json!({"effects": recorded, "outcome": outcome(result)}))
}

async fn execute(
    job: &Job,
    wire: &Wire<'_>,
    log: &mut LogContext<'_, '_>,
    image: &mut SuppliedMedia,
    max_depth: i64,
) -> Result<BflImageResponse, EditCallError> {
    let params = job.params()?;
    let extra_headers = job.extra_headers();
    let configured_key = job.configured.api_key.as_deref().map(SecretString::from);
    let mut call = EditCall {
        model: &job.model,
        prompt: job.prompt.as_deref(),
        image: Some(image),
        mask: MaskInput::Absent,
        optional_params: &job.optional_params,
        params,
        extra_headers: &extra_headers,
        timeout: job.timeout_millis.map(Duration::from_millis),
        // the oracle compares the sync handler; the async entry point keeps the same executor and transform
        aimage_edit: false,
        max_depth,
        configured_key: configured_key.as_ref(),
        now_seconds: 0,
    };
    let prepared = prepare_call(&mut call, log)?;
    let executor = EditExecutor {
        provider: wire,
        clock: wire,
        sleeper: wire,
    };
    let supplied: Option<SuppliedTransport<'_, Wire<'_>>> = None;
    executor.image_edit(&call, &prepared, supplied).await
}

fn max_depth() -> Result<i64, Box<HelperError>> {
    Ok(ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("")
    .or_config_error()?
    .sections
    .mistral_options
    .max_recurse_depth)
}

fn outcome(result: Result<BflImageResponse, EditCallError>) -> Value {
    match result {
        Ok(response) => {
            let urls: Vec<Option<String>> = response
                .data
                .iter()
                .map(|datum| datum.url.clone())
                .collect();
            json!({"ok": true, "urls": urls})
        }
        // LiteLLM's BFL error carries the status its callers branch on; the string is never compared
        Err(error) => json!({"ok": false, "status": error.status()}),
    }
}

fn media_from(spec: &ImageSpec) -> Result<SuppliedMedia, Box<HelperError>> {
    Ok(match spec {
        ImageSpec::Bytes { data } => {
            SuppliedMedia::Bytes(BASE64.decode(data).map_err(|error| failure(&error))?)
        }
        ImageSpec::List { items } => {
            let mut images = Vec::with_capacity(items.len());
            for item in items {
                images.push(SuppliedMedia::Bytes(
                    BASE64.decode(item).map_err(|error| failure(&error))?,
                ));
            }
            SuppliedMedia::List(images)
        }
    })
}
