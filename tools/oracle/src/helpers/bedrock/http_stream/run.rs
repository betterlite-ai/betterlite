use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use betterlite_bedrock::agentcore_stream::transport::StreamHandler;
use betterlite_bedrock::bedrock_http_stream::StreamDeps;
use betterlite_bedrock::bedrock_http_stream::converse::{ConverseCall, make_sync_call};
use betterlite_bedrock::bedrock_http_stream::invoke::{InvokeCall, make_call};
use betterlite_bedrock::bedrock_http_stream::stream::ChunkStream;
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_bedrock::invoke_events::DecoderClock;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::StreamExt as _;
use http::HeaderMap;
use serde_json::{Value, json};

use super::fixture::Fixture;
use super::support::{
    Counter, Quiet, SECONDS, TIMEOUT, env_time, failure, fixed_id, fixed_seconds, header_map,
    logging_state, sorted_pairs,
};
use super::types::{Case, Consume, Input, Route};
use super::views::{chunk_view, error_view, failure_view, log_view};
use betterlite_oracle_helper_api::error::HelperError;

pub(super) async fn execute(input: &Input) -> Result<Value, Box<HelperError>> {
    let mut views = Vec::with_capacity(input.cases.len());
    for case in &input.cases {
        views.push(one(case).await?);
    }
    Ok(json!({ "cases": views }))
}

async fn one(case: &Case) -> Result<Value, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?;
    let handler = Arc::new(Fixture::new(case)?);
    let mut state = logging_state(&redactor)?;
    let sink = Quiet;
    let logged = Mutex::new(Vec::new());
    let logger = |details: &Json| -> Result<(), PrePostError> {
        logged
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(details.clone());
        Ok(())
    };
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: Some(&logger),
        now: env_time()?,
    };
    let shared: Arc<dyn StreamHandler> = Arc::clone(&handler);
    let deps = StreamDeps {
        supplied: Some(Arc::clone(&shared)),
        default: Arc::clone(&shared),
        ids: Arc::new(Counter::new()),
        clock: DecoderClock {
            next_id: fixed_id,
            now: fixed_seconds,
        },
        redactor: &redactor,
        timeout: TIMEOUT,
        now: UNIX_EPOCH + Duration::from_secs(SECONDS),
    };
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let headers = header_map(&case.headers)?;
    let opened = match case.path {
        Route::ConverseSync => {
            make_sync_call(
                ConverseCall {
                    model: &case.model,
                    api_base: &case.api_base,
                    headers,
                    data: &case.data,
                    messages: &case.messages,
                    json_mode: case.json_mode,
                    fake_stream: case.fake_stream,
                },
                &deps,
                &mut log,
            )
            .await
        }
        Route::InvokeSync => make_call(invoke_case(case, headers, true), &deps, &mut log).await,
        Route::InvokeAsync => make_call(invoke_case(case, headers, false), &deps, &mut log).await,
    };
    let mut chunks = Vec::new();
    let mut failures = Vec::new();
    let mut response_headers = Value::Null;
    let mut error = None;
    match opened {
        Ok(mut stream) => {
            response_headers = json!(sorted_pairs(&stream.headers));
            drain(&mut stream.chunks, case.consume, &mut chunks, &mut failures).await;
        }
        Err(failure) => error = Some(failure),
    }
    drop(log);
    drop(env);
    drop(logger);
    let logs = logged.into_inner().unwrap_or_else(PoisonError::into_inner);
    Ok(json!({
        "post": handler.first_post(),
        "error": error.as_ref().map_or(Value::Null, |failure| error_view(failure, case)),
        "response_headers": response_headers,
        "chunks": chunks,
        "failures": failures,
        "logs": logs.iter().map(log_view).collect::<Vec<Value>>(),
    }))
}

fn invoke_case<'case>(case: &'case Case, headers: HeaderMap, sync: bool) -> InvokeCall<'case> {
    InvokeCall {
        model: &case.model,
        api_base: &case.api_base,
        headers,
        data: &case.data,
        // make_sync_call sends the signed body whenever it has one; make_call never sees it
        signed_json_body: match (sync, case.signed_json_body.as_deref()) {
            (true, Some(signed)) => Some(signed.as_bytes()),
            (true, None) | (false, _) => None,
        },
        messages: &case.messages,
        fake_stream: case.fake_stream,
        json_mode: case.json_mode,
        provider: case.provider.as_deref(),
        sync,
    }
}

async fn drain(
    chunks: &mut ChunkStream<'_>,
    consume: Consume,
    views: &mut Vec<Value>,
    failures: &mut Vec<Value>,
) {
    if consume == Consume::None {
        return;
    }
    while let Some(item) = chunks.next().await {
        match item {
            Ok(chunk) => views.push(chunk_view(chunk)),
            Err(failure) => failures.push(failure_view(&failure)),
        }
        if consume == Consume::First {
            return;
        }
    }
}
