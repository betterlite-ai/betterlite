use std::collections::BTreeMap;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::chat_response_stream::source::AsyncChunkStream;
use betterlite_core::api::messages_stream_split::errors::StreamFailure;
use betterlite_core::api::response_chat_dispatch::error::DispatchError;
use betterlite_core::api::response_chat_dispatch::handler::completion;
use betterlite_core::api::response_chat_dispatch::input::{BridgeCall, StreamStamps};
use betterlite_core::api::response_chat_dispatch::port::BridgeOutput;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use super::execution::{ExecutionFailure, RealExecution, env_flags};
use super::fixtures::{Fixture, caller_response};
use super::projection::{StreamViews, response_view};
use super::{Call, Outcome, RunOutcome};
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::Failure;

pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<Outcome, SdkHttpError> {
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        runs.push(run_once(run, api_base, env).await?);
    }
    Ok(Outcome { runs })
}

fn parse_run(run: &Json, api_base: &str) -> Result<IndexMap<String, Json>, SdkHttpError> {
    let Json::Object(fields) = run else {
        return Err(SdkHttpError::Invalid(String::from("a run is an object")));
    };
    let Some(Json::Object(mut kwargs)) = fields.get("kwargs").cloned() else {
        return Err(SdkHttpError::Invalid(String::from(
            "a run needs a kwargs object",
        )));
    };
    inject_api_base(&mut kwargs, api_base);
    let acompletion = matches!(fields.get("acompletion"), Some(Json::Bool(true)));
    kwargs.insert(String::from("acompletion"), Json::Bool(acompletion));
    Ok(kwargs)
}

// The caller never hands betterlite an api_base argument, so the mock origin joins the SDK params
fn inject_api_base(kwargs: &mut IndexMap<String, Json>, api_base: &str) {
    if let Some(Json::Object(params)) = kwargs.get_mut("litellm_params") {
        params.insert(String::from("api_base"), Json::Text(api_base.to_owned()));
    }
}

async fn run_once(
    run: &Json,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<RunOutcome, SdkHttpError> {
    let kwargs = parse_run(run, api_base)?;
    let fixture = Fixture::new("completion")?;
    let execution = RealExecution::new(api_base, env)?;
    let call = BridgeCall {
        kwargs: &kwargs,
        model_response: caller_response(),
        logging: fixture.state,
        client: Some(&()),
        limits: ApiLimits::default(),
        reasoning_auto_summary: env_flags(env).1,
        post_stream: None,
        stamps: StreamStamps::default(),
    };
    match completion(&execution, call).await {
        Ok(output) => project(output).await,
        Err(error) => Ok(RunOutcome {
            result: None,
            stream: None,
            failure: Some(failure_of(&error, &fixture.redactor)),
        }),
    }
}

async fn project(output: BridgeOutput) -> Result<RunOutcome, SdkHttpError> {
    match output {
        BridgeOutput::Response(response) => Ok(RunOutcome {
            result: Some(response_view(&response)),
            stream: None,
            failure: None,
        }),
        BridgeOutput::Chat(_chat) => Err(SdkHttpError::Invalid(String::from(
            "the bridge never passes a chat answer through",
        ))),
        BridgeOutput::Stream(stream) => Ok(RunOutcome {
            result: None,
            stream: Some(drain(Box::new(stream)).await?),
            failure: None,
        }),
    }
}

async fn drain(mut stream: Box<dyn AsyncChunkStream + Send>) -> Result<Json, SdkHttpError> {
    let mut views = StreamViews::default();
    while let Some(chunk) = stream.next_chunk().await.map_err(stream_error)? {
        views.push(&chunk);
    }
    Ok(views.finish())
}

fn stream_error(failure: StreamFailure) -> SdkHttpError {
    match failure {
        StreamFailure::Reported {
            status_code,
            message,
        } => SdkHttpError::Invalid(format!(
            "the responses stream reported {status_code}: {message}"
        )),
        StreamFailure::Unclassified { text } => SdkHttpError::Invalid(text),
    }
}

fn failure_of(error: &DispatchError<ExecutionFailure>, redactor: &Redactor) -> Failure {
    let DispatchError::Provider(source) = error else {
        return Failure {
            class: String::from("ValueError"),
            status: 0,
            kind: Some(None),
            param: Some(None),
            code: Some(None),
            message: error.to_string(),
        };
    };
    source.failure(redactor)
}
