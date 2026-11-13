use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::callback_logger::event::LoggerEvent;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::newrelic_metrics::flush::{async_send_batch, flush_queue};
use betterlite_logging::newrelic_metrics::logger::{NewRelicDeps, NewRelicMetricsLogger};
use betterlite_logging::newrelic_metrics::queue::push_record;
use betterlite_logging::newrelic_metrics::records::NewRelicMetricRecord;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::Deserialize;
use tokio::runtime::Builder;
use tokio_util::task::TaskTracker;

use crate::helpers::{Helper, HelperError, HelperResult};

mod report;
mod scripted;

use scripted::{ScriptedClock, ScriptedSleeps, ScriptedTransport};

const FLUSH_INTERVAL_SECONDS: u64 = 3600;
const SCRIPTED_KEY: &str = "team-key";

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Operation {
    SendBatch,
    Flush,
    Drain,
    PostStop,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default = "default_api_key")]
    api_key: String,
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    records: Vec<RecordInput>,
    #[serde(default)]
    during_send: Option<RecordInput>,
    #[serde(default)]
    replies: Vec<ReplyInput>,
    #[serde(default)]
    clock: Vec<f64>,
    #[serde(default)]
    payload: Option<StandardLoggingPayload>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordInput {
    team_id: String,
    team_alias: String,
    model_group: String,
    model: String,
    custom_llm_provider: String,
    status: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    response_cost: Decimal,
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    duration_ms: f64,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    team_max_budget: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    team_spend: Option<Decimal>,
    #[serde(default = "default_repeat")]
    repeat: usize,
}

impl RecordInput {
    fn record(&self) -> NewRelicMetricRecord {
        NewRelicMetricRecord {
            team_id: self.team_id.clone(),
            team_alias: self.team_alias.clone(),
            model_group: self.model_group.clone(),
            model: self.model.clone(),
            custom_llm_provider: self.custom_llm_provider.clone(),
            status: self.status.clone(),
            response_cost: self.response_cost,
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens: self.total_tokens,
            duration_ms: self.duration_ms,
            team_max_budget: self.team_max_budget,
            team_spend: self.team_spend,
        }
    }
}

#[derive(Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplyInput {
    #[serde(default = "default_status")]
    pub(super) status: u16,
    #[serde(default)]
    pub(super) network: bool,
}

fn default_api_key() -> String {
    SCRIPTED_KEY.to_owned()
}

const fn default_repeat() -> usize {
    1
}

const fn default_status() -> u16 {
    200
}

pub(super) fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "NewRelicMetricsError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(super) fn input_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InputError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(failure)?;
        runtime.block_on(run_scenario(input))
    }
}

fn seed_queue(logger: &NewRelicMetricsLogger, records: &[RecordInput]) {
    for record in records {
        for _copy in 0..record.repeat {
            push_record(logger, record.record());
        }
    }
}

async fn run_scenario(input: &Input) -> HelperResult {
    let during = input.during_send.as_ref().map(RecordInput::record);
    let transport = Arc::new(ScriptedTransport::new(&input.replies, during));
    let clock = Arc::new(ScriptedClock::new(&input.clock)?);
    let sleeps = Arc::new(ScriptedSleeps::default());
    let tasks = TaskTracker::new();
    let logger = NewRelicMetricsLogger::new(
        SecretString::from(input.api_key.clone()),
        input.region.as_deref(),
        NewRelicDeps {
            transport: Arc::<ScriptedTransport>::clone(&transport),
            clock: clock.handle(),
            sleeper: Arc::<ScriptedSleeps>::clone(&sleeps),
            flush_interval: Duration::from_secs(FLUSH_INTERVAL_SECONDS),
            tasks: tasks.clone(),
        },
    )
    .map_err(failure)?;
    transport.attach(&logger);
    seed_queue(&logger, &input.records);
    let (scheduled, delivered) = perform(input, &logger, &tasks).await?;
    report::scenario(&logger, &transport, &sleeps, scheduled, delivered)
}

async fn perform(
    input: &Input,
    logger: &Arc<NewRelicMetricsLogger>,
    tasks: &TaskTracker,
) -> Result<(Option<bool>, Option<bool>), Box<HelperError>> {
    match input.operation {
        Operation::SendBatch => Ok((None, Some(async_send_batch(logger).await))),
        Operation::Flush => {
            flush_queue(logger).await;
            Ok((None, None))
        }
        Operation::Drain => {
            let scheduled = logger.stop();
            tasks.close();
            tasks.wait().await;
            Ok((Some(scheduled), None))
        }
        Operation::PostStop => {
            let scheduled = logger.stop();
            tasks.close();
            tasks.wait().await;
            deliver_event(input, logger).await?;
            Ok((Some(scheduled), None))
        }
    }
}

async fn deliver_event(
    input: &Input,
    logger: &NewRelicMetricsLogger,
) -> Result<(), Box<HelperError>> {
    let payload = input
        .payload
        .as_ref()
        .ok_or_else(|| input_error("the post_stop operation needs a payload"))?;
    let event = LoggerEvent {
        payload: Arc::new(payload.clone()),
        dynamic_params: Arc::new(Json::Object(IndexMap::new())),
        litellm_metadata: Arc::new(IndexMap::new()),
        stream: false,
        guardrail_snapshot: None,
    };
    logger
        .async_log_success_event(Arc::new(event))
        .await
        .map_err(failure)
}
