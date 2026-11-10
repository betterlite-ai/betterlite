use std::sync::Arc;

use betterlite_prompts::arguments::json::{Json, Object};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::{HeaderMap, Method};

use super::super::error::{ReplicateError, ReplicateFailure};
use super::super::reply::{join_output, json_display};
use super::completion::Replicate;
use super::transport::{REQUEST_TIMEOUT, ReplicateRequest, ReplicateResponse, ReplicateTransport};
use super::{OutputPolicy, ReplicateChunk, ReplicateSleeper, ReplicateStream};

struct PollState {
    transport: Arc<dyn ReplicateTransport>,
    sleeper: Arc<dyn ReplicateSleeper>,
    url: String,
    headers: HeaderMap,
    polling_delay_seconds: f64,
    policy: OutputPolicy,
    previous_output: String,
    status: String,
    pending_error: Option<ReplicateFailure>,
}

pub fn poll_stream(replicate: &Replicate<'_>, url: String, headers: HeaderMap) -> ReplicateStream {
    let state = PollState {
        transport: Arc::clone(&replicate.transport),
        sleeper: Arc::clone(&replicate.sleeper),
        url,
        headers,
        polling_delay_seconds: replicate.polling.polling_delay_seconds,
        policy: replicate.policy,
        previous_output: String::new(),
        status: String::new(),
        pending_error: None,
    };
    stream::try_unfold(state, next_chunk).boxed()
}

async fn poll_once(state: &PollState) -> Result<ReplicateResponse, ReplicateFailure> {
    state.sleeper.sleep(state.polling_delay_seconds).await;
    state
        .transport
        .send(ReplicateRequest {
            method: Method::GET,
            url: state.url.clone(),
            headers: state.headers.clone(),
            body: None,
            timeout: REQUEST_TIMEOUT,
        })
        .await
        .map_err(|source| ReplicateFailure::Transport { source })
}

fn failed_prediction(
    object: &Object,
    headers: &HeaderMap,
) -> Result<Option<ReplicateFailure>, ReplicateFailure> {
    if !matches!(object.get("status"), Some(Json::Text(status)) if status == "failed") {
        return Ok(None);
    }
    let error = match object.get("error") {
        None => String::new(),
        Some(error) => json_display(error)?,
    };
    Ok(Some(ReplicateFailure::Api(Box::new(ReplicateError {
        status_code: 400,
        message: format!("Error: {error}"),
        headers: Some(headers.clone()),
    }))))
}

fn handle_reply(
    state: &mut PollState,
    response: &ReplicateResponse,
) -> Result<Option<ReplicateChunk>, ReplicateFailure> {
    let value: Json = serde_json::from_slice(&response.body)
        .map_err(|source| ReplicateFailure::Decode { source })?;
    let Some(object) = value.as_object() else {
        return Err(ReplicateFailure::ReplyNotObject);
    };
    let Some(Json::Text(status)) = object.get("status") else {
        return Err(ReplicateFailure::MissingStatus);
    };
    state.status.clone_from(status);

    // LiteLLM yields the output chunk of a failed prediction before it raises, so the failure waits for the next poll
    let failure = failed_prediction(object, &response.headers)?;
    let Some(output) = object.get("output") else {
        return failure.map_or(Ok(None), Err);
    };
    if state.policy == OutputPolicy::Lenient && matches!(output, Json::Null) {
        return failure.map_or(Ok(None), Err);
    }
    let joined = join_output(output, state.policy, &response.headers)?;
    let already_sent = state.previous_output.chars().count();
    let new_output: String = joined.chars().skip(already_sent).collect();
    state.previous_output = joined;
    state.pending_error = failure;
    Ok(Some(ReplicateChunk {
        output: new_output,
        status: state.status.clone(),
    }))
}

async fn next_chunk(
    mut state: PollState,
) -> Result<Option<(ReplicateChunk, PollState)>, ReplicateFailure> {
    if let Some(error) = state.pending_error.take() {
        return Err(error);
    }
    loop {
        if matches!(state.status.as_str(), "succeeded" | "failed" | "canceled") {
            return Ok(None);
        }
        let response = poll_once(&state).await?;
        // a non-200 poll only logs in LiteLLM and polling goes on, with no attempt limit
        if response.status != 200 {
            continue;
        }
        let Some(chunk) = handle_reply(&mut state, &response)? else {
            continue;
        };
        return Ok(Some((chunk, state)));
    }
}
