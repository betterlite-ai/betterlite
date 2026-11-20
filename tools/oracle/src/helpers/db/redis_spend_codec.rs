use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::redis_spend::buffer::{RedisUpdateBuffer, SpendBufferPipeline};
use betterlite_db::redis_spend::codec::decode_log_record;
use betterlite_db::redis_spend::events::NoRedisBufferEventSink;
use betterlite_db::redis_spend::keys::{REDIS_DAILY_TAG_BUFFER_KEY, REDIS_SPEND_LOGS_BUFFER_KEY};
use betterlite_db::redis_spend::lists::{
    BufferPop, BufferReply, SpendBufferLists, SpendListFuture,
};
use betterlite_db::redis_spend::log_row::{SpendLogRow, SpendLogValue};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use tokio::runtime::Builder;

// MAX_REDIS_BUFFER_DEQUEUE_COUNT's default; a scripted pop answers the same whatever count it is asked for.
const DEQUEUE_COUNT: i64 = 100;
// get_spend_logs_from_redis_buffer takes its limit from the caller, and a scripted pop ignores it.
const SPEND_LOG_LIMIT: i64 = 10;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    enabled: bool,
    attached: bool,
    park: Option<ParkStep>,
    drain: Option<DrainStep>,
    pipeline: Option<PipelineStep>,
    tags: Option<TagsStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParkStep {
    rows: Vec<RowInput>,
}

// A spend log row as a case spells it: the JSON columns, plus the columns whose value is a datetime.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RowInput {
    values: BTreeMap<String, Value>,
    datetimes: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DrainStep {
    reply: Reply,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TagsStep {
    reply: Reply,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PipelineStep {
    replies: Vec<Reply>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Reply {
    Missing,
    Scalar { value: String },
    Items { values: Vec<String> },
}

impl Reply {
    fn buffer(&self) -> BufferReply {
        match self {
            Self::Missing => BufferReply::Missing,
            Self::Scalar { value } => BufferReply::Scalar(value.clone().into_bytes()),
            Self::Items { values } => BufferReply::List(
                values
                    .iter()
                    .map(|value| value.clone().into_bytes())
                    .collect(),
            ),
        }
    }
}

// One scripted pop per queue key, and one scripted pipeline reply for the seven-queue drain.
#[derive(Default)]
struct ScriptedLists {
    pops: Mutex<BTreeMap<String, BufferReply>>,
    pipeline: Mutex<Option<Vec<BufferReply>>>,
    pushed: Mutex<Vec<String>>,
}

impl ScriptedLists {
    fn script_pop(&self, key: &str, reply: BufferReply) {
        self.pops
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), reply);
    }

    fn script_pipeline(&self, replies: Vec<BufferReply>) {
        *self.pipeline.lock().unwrap_or_else(PoisonError::into_inner) = Some(replies);
    }

    fn pushed(&self) -> Vec<String> {
        self.pushed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SpendBufferLists for ScriptedLists {
    fn pop<'call>(
        &'call self,
        key: &'call str,
        _count: Option<i64>,
    ) -> SpendListFuture<'call, BufferReply> {
        Box::pin(async move {
            let pops = self.pops.lock().unwrap_or_else(PoisonError::into_inner);
            Ok(pops.get(key).cloned().unwrap_or(BufferReply::Missing))
        })
    }

    fn pop_pipeline<'call>(
        &'call self,
        _operations: &'call [BufferPop],
    ) -> SpendListFuture<'call, Vec<BufferReply>> {
        Box::pin(async move {
            let pipeline = self.pipeline.lock().unwrap_or_else(PoisonError::into_inner);
            Ok(pipeline.clone().unwrap_or_default())
        })
    }

    fn push_and_trim<'call>(
        &'call self,
        _key: &'call str,
        values: &'call [String],
        _max_len: i64,
    ) -> SpendListFuture<'call, i64> {
        Box::pin(async move {
            let mut pushed = self.pushed.lock().unwrap_or_else(PoisonError::into_inner);
            pushed.extend(values.iter().cloned());
            Ok(i64::try_from(pushed.len()).unwrap_or(i64::MAX))
        })
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(failure)?;
        let output = runtime.block_on(scenario(input))?;
        serde_json::to_value(output).map_err(failure)
    }
}

fn script_replies(lists: &ScriptedLists, input: &Input) {
    if let Some(step) = &input.drain {
        lists.script_pop(REDIS_SPEND_LOGS_BUFFER_KEY, step.reply.buffer());
    }
    if let Some(step) = &input.tags {
        lists.script_pop(REDIS_DAILY_TAG_BUFFER_KEY, step.reply.buffer());
    }
    if let Some(step) = &input.pipeline {
        let replies = step.replies.iter().map(Reply::buffer).collect();
        lists.script_pipeline(replies);
    }
}

async fn scenario(input: &Input) -> Result<BTreeMap<String, Value>, Box<HelperError>> {
    let lists = Arc::new(ScriptedLists::default());
    script_replies(&lists, input);
    let port: Arc<dyn SpendBufferLists> = Arc::clone(&lists);
    let buffer = RedisUpdateBuffer {
        lists: if input.attached { Some(port) } else { None },
        enabled: input.enabled,
        dequeue_count: DEQUEUE_COUNT,
        events: Arc::new(NoRedisBufferEventSink),
    };
    let mut output = BTreeMap::new();
    if let Some(step) = &input.park {
        output.insert("parked".to_owned(), parked(&buffer, &lists, step).await?);
    }
    if input.drain.is_some() {
        output.insert("drained".to_owned(), drained(&buffer).await?);
    }
    if input.pipeline.is_some() {
        output.insert("pipeline".to_owned(), pipelined(&buffer).await);
    }
    if input.tags.is_some() {
        output.insert("tags".to_owned(), tagged(&buffer).await?);
    }
    Ok(output)
}

// store_spend_logs_in_redis: the boolean the caller falls back on, with the exact rows the push carried.
async fn parked(
    buffer: &RedisUpdateBuffer,
    lists: &ScriptedLists,
    step: &ParkStep,
) -> Result<Value, Box<HelperError>> {
    let mut rows = Vec::with_capacity(step.rows.len());
    for row in &step.rows {
        rows.push(spend_log_row(row)?);
    }
    let parked = buffer.park_spend_logs(&rows).await;
    let mut values = Vec::new();
    for encoded in lists.pushed() {
        values.push(serde_json::from_str::<Value>(&encoded).map_err(failure)?);
    }
    Ok(serde_json::json!({ "parked": parked, "values": values }))
}

// A row is built from its JSON columns, then each datetime column becomes the isoformat text parking writes.
fn spend_log_row(row: &RowInput) -> Result<SpendLogRow, Box<HelperError>> {
    let encoded = serde_json::to_vec(&row.values).map_err(failure)?;
    let Some(mut decoded) = decode_log_record(&encoded).map_err(failure)? else {
        return Err(failure("a spend log row must be a JSON object"));
    };
    for (field, stamp) in &row.datetimes {
        let moment = stamp
            .parse::<DateTime<Utc>>()
            .map_err(|error| failure(format!("the {field} column is not a date-time: {error}")))?;
        decoded.insert(field.clone(), SpendLogValue::from(moment));
    }
    Ok(decoded)
}

// get_spend_logs_from_redis_buffer: a parse failure is the invalid marker the source raises from.
async fn drained(buffer: &RedisUpdateBuffer) -> Result<Value, Box<HelperError>> {
    match buffer.drain_spend_logs(SPEND_LOG_LIMIT).await {
        Ok(rows) => serde_json::to_value(rows).map_err(failure),
        Err(_) => Ok(serde_json::json!({ "invalid": true })),
    }
}

// get_all_transactions_from_redis_buffer_pipeline, named by slot so the seven queues stay distinguishable.
async fn pipelined(buffer: &RedisUpdateBuffer) -> Value {
    match buffer.drain_pipeline().await {
        Ok(pipeline) => slots(&pipeline),
        Err(_) => serde_json::json!({ "invalid": true }),
    }
}

fn slots(pipeline: &SpendBufferPipeline) -> Value {
    serde_json::json!({
        "entity": &pipeline.entity,
        "daily_user": &pipeline.daily_user,
        "daily_team": &pipeline.daily_team,
        "daily_org": &pipeline.daily_org,
        "daily_end_user": &pipeline.daily_end_user,
        "daily_agent": &pipeline.daily_agent,
        "window": &pipeline.window,
    })
}

// get_all_daily_tag_spend_update_transactions_from_redis_buffer, which the seven-slot pipeline leaves alone.
async fn tagged(buffer: &RedisUpdateBuffer) -> Result<Value, Box<HelperError>> {
    match buffer.drain_daily_tags().await {
        Ok(tags) => serde_json::to_value(tags).map_err(failure),
        Err(_) => Ok(serde_json::json!({ "invalid": true })),
    }
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: Display,
{
    Box::new(HelperError {
        class: "RedisSpendCodec".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
