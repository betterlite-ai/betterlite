use std::mem::take;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_cache::cache_key::value::KeyObject;
use betterlite_cache::replay::CachedDict;
use betterlite_cache::write_completion::tracker::CacheWriteTracker;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_sse_events::stream::MessagesProviderStream as _;
use betterlite_core::api::messages_stream_restamp::cache_stream::{
    MessagesStreamCacheDeps, MessagesStreamCacheTarget, MessagesStreamCacheWriter,
};
use betterlite_core::api::messages_stream_restamp::replay::{
    CachedMessagesResult, convert_cached_messages_result,
};
use betterlite_core::api::messages_stream_restamp::{
    MessagesStreamChunk, MessagesStreamRestampInput, restamp_messages_stream,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::subscriber;
use tracing_subscriber::layer::SubscriberExt as _;

use self::doubles::{
    RecordingCache, RecordingLogging, ScriptedStream, cache_settings, call_context, chunk_value,
    writes_value,
};

mod capture;
mod doubles;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    restamp: Option<RestampInput>,
    cache_stream: Option<CacheStreamInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestampInput {
    requested_model: String,
    chunks: Vec<ChunkSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ChunkSpec {
    Event { value: Value },
    Bytes { value: String },
    Text { value: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheStreamInput {
    reads: Vec<String>,
    preset_cache_key: Option<String>,
    fail: Option<String>,
    replay: bool,
}

struct Recorded {
    yielded: Vec<String>,
    writes: Vec<(Value, KeyObject)>,
    replayed: Option<Vec<String>>,
    cache_hit: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(json!({
            "restamp": input.restamp.as_ref().map(restamp_answer).transpose()?,
            "cache_stream": input.cache_stream.as_ref().map(cache_stream_answer).transpose()?,
        }))
    }
}

fn restamp_answer(input: &RestampInput) -> Result<Value, Box<HelperError>> {
    let chunks = input
        .chunks
        .iter()
        .map(chunk_from_spec)
        .collect::<Result<Vec<_>, _>>()?;
    let result = restamp_messages_stream(&MessagesStreamRestampInput {
        requested_model: input.requested_model.clone(),
        chunks,
    });
    Ok(json!({
        "chunks": result.chunks.iter().map(chunk_value).collect::<Vec<_>>(),
        "flushed": chunk_value(&result.flushed),
    }))
}

fn chunk_from_spec(spec: &ChunkSpec) -> Result<MessagesStreamChunk, Box<HelperError>> {
    match spec {
        ChunkSpec::Event { value } => {
            let event: Json = serde_json::from_value(value.clone()).or_value_error()?;
            Ok(MessagesStreamChunk::Event(event))
        }
        ChunkSpec::Bytes { value } => BASE64
            .decode(value)
            .or_value_error()
            .map(MessagesStreamChunk::Bytes),
        ChunkSpec::Text { value } => Ok(MessagesStreamChunk::Text(value.clone())),
    }
}

fn cache_stream_answer(input: &CacheStreamInput) -> Result<Value, Box<HelperError>> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let cache = Arc::new(RecordingCache::new(input.fail.clone()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let guard = subscriber::set_default(
        tracing_subscriber::registry().with(capture::Capture(Arc::clone(&events))),
    );
    let attempt = runtime.block_on(write_stream(input, Arc::clone(&cache)));
    drop(guard);
    let recorded = attempt?;
    let captured = events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(json!({
        "yielded": recorded.yielded,
        "writes": writes_value(&recorded.writes),
        "events": captured,
        "replayed": recorded.replayed,
        "cache_hit": recorded.cache_hit,
    }))
}

async fn write_stream(
    input: &CacheStreamInput,
    cache: Arc<RecordingCache>,
) -> Result<Recorded, Box<HelperError>> {
    let tracker = Arc::new(CacheWriteTracker {
        tasks: TaskTracker::new(),
        cancellation: CancellationToken::new(),
    });
    let target: Arc<dyn MessagesStreamCacheTarget> = Arc::<RecordingCache>::clone(&cache);
    let reads = input
        .reads
        .iter()
        .map(|read| read.as_bytes().to_vec())
        .collect();
    let mut writer = MessagesStreamCacheWriter::new(
        ScriptedStream::new(reads),
        cache_settings(input.preset_cache_key.clone()),
        MessagesStreamCacheDeps {
            cache: Some(target),
            tracker: Arc::clone(&tracker),
            call: call_context(),
            limits: ApiLimits::default(),
        },
    );
    let yielded = drain(&mut writer).await?;
    tracker.wait_idle().await;
    let stored = take(&mut *cache.writes.lock().unwrap_or_else(PoisonError::into_inner));
    let (replayed, cache_hit) = replay_written(input.replay, &stored).await?;
    Ok(Recorded {
        yielded: yielded
            .into_iter()
            .map(|chunk| String::from_utf8(chunk).or_value_error())
            .collect::<Result<Vec<_>, _>>()?,
        writes: stored,
        replayed,
        cache_hit,
    })
}

async fn drain(
    writer: &mut MessagesStreamCacheWriter<ScriptedStream>,
) -> Result<Vec<Vec<u8>>, Box<HelperError>> {
    let mut yielded = Vec::new();
    while let Some(chunk) = writer.next_chunk().await.or_value_error()? {
        yielded.push(chunk);
    }
    Ok(yielded)
}

async fn replay_written(
    replay: bool,
    writes: &[(Value, KeyObject)],
) -> Result<(Option<Vec<String>>, bool), Box<HelperError>> {
    if !replay {
        return Ok((None, false));
    }
    let Some((payload, _)) = writes.first() else {
        return Ok((None, false));
    };
    let cached: CachedDict = serde_json::from_value(payload.clone()).or_value_error()?;
    let logging = RecordingLogging::default();
    let converted = convert_cached_messages_result(cached, logging.clone()).or_value_error()?;
    let CachedMessagesResult::Stream(mut iterator) = converted else {
        return Ok((None, logging.stamped.load(Ordering::SeqCst)));
    };
    let mut events = Vec::new();
    while let Some(chunk) = iterator.next_chunk().await.or_value_error()? {
        events.push(String::from_utf8(chunk).or_value_error()?);
    }
    Ok((Some(events), logging.stamped.load(Ordering::SeqCst)))
}
