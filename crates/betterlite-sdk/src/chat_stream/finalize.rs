use betterlite_logging::fields::usage::Usage as LogUsage;
use betterlite_logging::fields::usage::project::usage_json;
use betterlite_logging::logging_streams::assembly::StreamBuild;
use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::creation::{
    model_response_creator, stamp_served_service_tier,
};
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, StreamChunk};
use betterlite_streaming::stream_chunk_shaping::tokens::finish_reason_handler;
use betterlite_streaming::stream_chunk_shaping::usage_totals::propagate_usage_cost;

use super::ChatStream;
use super::config::CallMode;
use super::error::ChatStreamCause;
use super::{Lifecycle, chunk, json, mcp};

// The three stream_chunk_builder results go through B252, which hands back what it built.
pub(super) async fn end_of_source<S>(
    stream: &mut ChatStream<'_, S>,
) -> Result<Option<StreamChunk>, ChatStreamCause> {
    if stream.shaper.progress.sent_last_chunk {
        return handoff(stream).await;
    }
    synthesized_finish(stream)
}

// __next__ and __anext__ both invent a finish chunk when the source ends without a finish reason.
fn synthesized_finish<S>(
    stream: &mut ChatStream<'_, S>,
) -> Result<Option<StreamChunk>, ChatStreamCause> {
    stream.shaper.progress.sent_last_chunk = true;
    let mut response = finish_reason_handler(&mut stream.shaper);
    if stream.config.stream_options.is_none() {
        chunk::insert_total_usage(&mut response, &stream.shaper.chunks)?;
    }
    let logging =
        matches!(stream.config.mode, CallMode::Blocking) && !stream.config.logging.disable;
    if logging {
        let logged: Json = json::bridge(&response)?;
        stream
            .ports
            .logging
            .log_success(Some(logged), stream.cache_hit);
    }
    Ok(Some(response))
}

// The finished stream: cache the complete response, log it, then answer with the usage chunk if asked.
async fn handoff<S>(
    stream: &mut ChatStream<'_, S>,
) -> Result<Option<StreamChunk>, ChatStreamCause> {
    let built = stream.ports.logging.completed(&stream.shaper.chunks);
    let mut response = model_response_creator(&mut stream.shaper, None, None)?;
    stream.lifecycle = Lifecycle::Completed;
    let payload = assembled_payload(stream, built, &mut response)?;
    if let Some(complete) = payload {
        store_and_log(stream, &complete).await;
    } else {
        log_without_assembly(stream, &response)?;
    }
    if stream.config.send_stream_usage {
        return Ok(Some(response));
    }
    Ok(None)
}

fn assembled_payload<S>(
    stream: &mut ChatStream<'_, S>,
    built: StreamBuild,
    response: &mut StreamChunk,
) -> Result<Option<Json>, ChatStreamCause> {
    match built {
        StreamBuild::Complete(complete) => Ok(Some(complete_payload(stream, complete, response)?)),
        // model_response_creator's failure only drops the fallback, as LiteLLM's except Exception does.
        StreamBuild::UsageOnly(usage) => Ok(usage_only_payload(stream, &usage, response).ok()),
        StreamBuild::Empty => Ok(None),
    }
}

// The assembled response read through the chunk shape: usage, service tier and hidden params live there.
fn complete_payload<S>(
    stream: &ChatStream<'_, S>,
    complete: Json,
    response: &mut StreamChunk,
) -> Result<Json, ChatStreamCause> {
    let mut view: StreamChunk = json::bridge(&complete)?;
    let before = view.hidden_params.clone();
    let providers = [String::from("openrouter")];
    propagate_usage_cost(&mut view, stream.config.provider.as_deref(), &providers);
    response.usage.clone_from(&view.usage);
    stamp_served_service_tier(response, &view);
    if view.hidden_params == before {
        return Ok(complete);
    }
    let hidden: Json = json::bridge(&view.hidden_params)?;
    Ok(with_hidden_params(complete, hidden))
}

// A builder failure keeps the cost tracked through a usage-only response built from the chunks.
fn usage_only_payload<S>(
    stream: &mut ChatStream<'_, S>,
    usage: &LogUsage,
    response: &mut StreamChunk,
) -> Result<Json, ChatStreamCause> {
    let total = usage_json(usage);
    let mut arguments = JsonObject::new();
    arguments.insert("usage".to_owned(), json::bridge(&total)?);
    let mut fallback = model_response_creator(&mut stream.shaper, Some(&arguments), None)?;
    let providers = [String::from("openrouter")];
    propagate_usage_cost(&mut fallback, stream.config.provider.as_deref(), &providers);
    response.usage.clone_from(&fallback.usage);
    stamp_served_service_tier(response, &fallback);
    json::bridge(&fallback)
}

fn with_hidden_params(mut complete: Json, hidden: Json) -> Json {
    if let Json::Object(fields) = &mut complete {
        fields.insert("hidden_params".to_owned(), hidden);
    }
    complete
}

async fn store_and_log<S>(stream: &ChatStream<'_, S>, complete: &Json) {
    if !stream.cache_hit
        && let Some(cache) = stream.ports.cache
    {
        cache.store(complete.clone()).await;
    }
    if !stream.config.logging.disable {
        stream
            .ports
            .logging
            .log_success(Some(complete.clone()), stream.cache_hit);
    }
}

// Complete logging in __next__ keeps the empty creator chunk; __anext__ dispatches None instead.
fn log_without_assembly<S>(
    stream: &ChatStream<'_, S>,
    response: &StreamChunk,
) -> Result<(), ChatStreamCause> {
    if stream.config.logging.disable {
        return Ok(());
    }
    let logged = match stream.config.mode {
        CallMode::Blocking => Some(json::bridge(response)?),
        CallMode::Async => None,
    };
    stream.ports.logging.log_success(logged, stream.cache_hit);
    Ok(())
}
