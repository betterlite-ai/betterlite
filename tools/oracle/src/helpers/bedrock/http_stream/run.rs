use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_bedrock::agentcore_stream::transport::StreamHandler;
use betterlite_bedrock::bedrock_http_stream::StreamDeps;
use betterlite_bedrock::bedrock_http_stream::converse::{ConverseCall, make_sync_call};
use betterlite_bedrock::bedrock_http_stream::error::HttpStreamError;
use betterlite_bedrock::bedrock_http_stream::invoke::{InvokeCall, make_call};
use betterlite_bedrock::bedrock_http_stream::stream::{ChunkStream, OpenedStream};
use betterlite_bedrock::image_call::plan::LogContext;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::StreamExt as _;
use http::HeaderMap;
use serde_json::{Value, json};

use super::fixture::Fixture;
use super::support::{
    Quiet, failure, header_map, logging_state, pre_post_env, sorted_pairs, stream_deps,
};
use super::types::{Case, Consume, Input, Route};
use super::views::{chunk_view, error_view, log_view};
use betterlite_oracle_helper_api::error::HelperError;

struct Collected {
    response_headers: Value,
    chunks: Vec<Value>,
    failures: Vec<Value>,
    error: Option<HttpStreamError<Infallible>>,
}

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
    let env = pre_post_env(&redactor, &sink, &logger)?;
    let shared: Arc<dyn StreamHandler> = Arc::clone(&handler);
    let deps = stream_deps(shared, &redactor);
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let result = collected(case, &deps, &mut log).await?;
    drop(log);
    drop(env);
    drop(logger);
    let logs = logged.into_inner().unwrap_or_else(PoisonError::into_inner);
    Ok(json!({
        "post": handler.first_post(),
        "error": result.error.as_ref().map_or(Value::Null, |failure| error_view(failure, case)),
        "response_headers": result.response_headers,
        "chunks": result.chunks,
        "failures": result.failures,
        "logs": logs.iter().map(log_view).collect::<Vec<Value>>(),
    }))
}

// The helper call the case names, at the seam its route reaches.
async fn open_route<'deps>(
    case: &Case,
    headers: HeaderMap,
    deps: &StreamDeps<'deps>,
    log: &mut LogContext<'_, '_>,
) -> Result<OpenedStream<'deps>, HttpStreamError<Infallible>> {
    match case.path {
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
                deps,
                log,
            )
            .await
        }
        Route::InvokeSync => make_call(invoke_case(case, headers, true), deps, log).await,
        Route::InvokeAsync => make_call(invoke_case(case, headers, false), deps, log).await,
    }
}

// The opened stream read exactly as far as the case asks.
async fn collected(
    case: &Case,
    deps: &StreamDeps<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<Collected, Box<HelperError>> {
    let headers = header_map(&case.headers)?;
    let opened = open_route(case, headers, deps, log).await;
    let mut result = Collected {
        response_headers: Value::Null,
        chunks: Vec::new(),
        failures: Vec::new(),
        error: None,
    };
    match opened {
        Ok(mut stream) => {
            result.response_headers = json!(sorted_pairs(&stream.headers));
            drain(
                &mut stream.chunks,
                case.consume,
                &mut result.chunks,
                &mut result.failures,
            )
            .await;
        }
        Err(failure) => result.error = Some(failure),
    }
    Ok(result)
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
            Err(failure) => failures.push(json!({ "status": failure.status() })),
        }
        if consume == Consume::First {
            return;
        }
    }
}
