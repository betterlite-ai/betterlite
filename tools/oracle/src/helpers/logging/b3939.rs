mod capture;
mod scripted;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_logging::batch::logger::FlushReport;
use betterlite_logging::datadog::metrics::datadog_metrics::{DatadogMetricsLogger, MetricsDeps};
use betterlite_logging::datadog::metrics::flush::flush_queue;
use betterlite_logging::datadog::metrics::health::async_health_check;
use betterlite_logging::datadog::metrics::measurements::{
    MetricSource, RequestTiming, SUCCESS_STATUS_CODE, failure_status_code, metrics_from_log,
};
use betterlite_logging::datadog::metrics::settings::MetricsInit;
use betterlite_logging::datadog::metrics::tags::extract_tags;
use betterlite_logging::datadog_logging::transport::Transport;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use self::capture::WarningCounter;
use self::scripted::{FixedClock, ScriptedTransport};
use crate::helpers::{Helper, HelperError, HelperResult};

const DEFAULT_CLOCK: i64 = 1_700_000_000;
const NANOS_PER_SECOND: i64 = 1_000_000_000;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Init,
    AddMetrics,
    SuccessEvent,
    FailureEvent,
    Flush,
    HealthCheck,
    ExtractTags,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    payload: Option<StandardLoggingPayload>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    start_time: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    end_time: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    api_call_start_time: Option<Decimal>,
    #[serde(default)]
    status_code: Option<String>,
    #[serde(default)]
    batch_size: Option<usize>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    clock: Option<Decimal>,
    #[serde(default)]
    replies: Vec<ReplyInput>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplyInput {
    pub(super) status: u16,
}

fn duration_of(seconds: Decimal) -> Result<Duration, Box<HelperError>> {
    let signed = seconds
        .checked_mul(Decimal::from(NANOS_PER_SECOND))
        .and_then(|scaled| scaled.to_i64())
        .ok_or_else(|| input_error("the clock does not fit a duration"))?;
    let nanos =
        u64::try_from(signed).map_err(|_| input_error("the clock is before the unix epoch"))?;
    Ok(Duration::from_nanos(nanos))
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "DatadogMetricsError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn input_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: message.to_owned(),
    })
}

fn payload_of(input: &Input) -> Result<&StandardLoggingPayload, Box<HelperError>> {
    input
        .payload
        .as_ref()
        .ok_or_else(|| input_error("the operation needs a payload"))
}

const fn timing_of(input: &Input) -> RequestTiming {
    RequestTiming {
        start_time: input.start_time,
        end_time: input.end_time,
        api_call_start_time: input.api_call_start_time,
    }
}

const fn source<'source>(
    input: &Input,
    logger: &'source DatadogMetricsLogger,
    payload: &'source StandardLoggingPayload,
    status_code: String,
) -> MetricSource<'source> {
    MetricSource {
        environment: &logger.settings.environment,
        payload,
        timing: timing_of(input),
        status_code,
    }
}

fn seed(input: &Input, logger: &DatadogMetricsLogger) -> Result<(), Box<HelperError>> {
    let payload = payload_of(input)?;
    let status_code = input
        .status_code
        .clone()
        .unwrap_or_else(|| SUCCESS_STATUS_CODE.to_owned());
    let source = source(input, logger, payload, status_code);
    let series = metrics_from_log(
        &source,
        logger.settings.batch.flush_interval.as_secs(),
        input.clock.unwrap_or_else(|| Decimal::from(DEFAULT_CLOCK)),
    )
    .map_err(failure)?;
    for entry in series {
        let _enqueued = logger.queue.enqueue(entry);
    }
    Ok(())
}

fn flush_report(outcome: &FlushReport) -> Value {
    json!({
        "batch": outcome.batch,
        "delivered": outcome.delivered,
        "requeued": outcome.requeued,
    })
}

async fn perform(input: &Input, logger: &DatadogMetricsLogger) -> HelperResult {
    let mut health = Value::Null;
    let mut report = Value::Null;
    let mut tags = Value::Null;
    match input.operation {
        Operation::Init => {}
        Operation::AddMetrics => seed(input, logger)?,
        Operation::SuccessEvent => {
            let payload = payload_of(input)?;
            let source = source(input, logger, payload, SUCCESS_STATUS_CODE.to_owned());
            logger.record(&source).await;
        }
        Operation::FailureEvent => {
            let payload = payload_of(input)?;
            let status_code = failure_status_code(payload);
            let source = source(input, logger, payload, status_code);
            logger.record(&source).await;
        }
        Operation::ExtractTags => {
            let payload = payload_of(input)?;
            tags = json!(extract_tags(
                &logger.settings.environment,
                payload,
                input.status_code.as_deref()
            ));
        }
        Operation::Flush => {
            seed(input, logger)?;
            report = flush_report(&flush_queue(logger).await);
        }
        Operation::HealthCheck => {
            health = serde_json::to_value(async_health_check(logger).await).map_err(failure)?;
        }
    }
    Ok(json!({"health": health, "report": report, "tags": tags}))
}

struct Fixture {
    logger: Arc<DatadogMetricsLogger>,
    transport: Arc<ScriptedTransport>,
    warnings: WarningCounter,
}

fn fixture(input: &Input) -> Result<Fixture, Box<HelperError>> {
    let warnings = WarningCounter::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    let transport = ScriptedTransport::new(&input.replies);
    let concrete: Arc<ScriptedTransport> = Arc::clone(&transport);
    let transport_trait: Arc<dyn Transport> = concrete;
    let clock = FixedClock(duration_of(
        input.clock.unwrap_or_else(|| Decimal::from(DEFAULT_CLOCK)),
    )?);
    let logger = with_default(subscriber, || {
        DatadogMetricsLogger::new(
            &input.environment,
            &MetricsInit {
                batch_size: input.batch_size,
            },
            MetricsDeps {
                transport: transport_trait,
                clock: Arc::new(clock),
                tasks: None,
            },
        )
    })
    .map_err(failure)?;
    Ok(Fixture {
        logger,
        transport,
        warnings,
    })
}

fn run_scenario(input: &Input) -> HelperResult {
    let fixture = fixture(input)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)?;
    let outcome = runtime.block_on(perform(input, &fixture.logger))?;
    let queued = serde_json::to_value(fixture.logger.queue.pending()).map_err(failure)?;
    let mut scenario = outcome;
    let Some(fields) = scenario.as_object_mut() else {
        return Err(input_error("the scenario outcome is not an object"));
    };
    let settings = &fixture.logger.settings;
    fields.insert("series".to_owned(), queued);
    fields.insert("requests".to_owned(), json!(fixture.transport.calls()));
    fields.insert(
        "warnings".to_owned(),
        json!(fixture.warnings.0.load(Ordering::Relaxed)),
    );
    fields.insert("upload_url".to_owned(), json!(settings.upload_url.clone()));
    fields.insert(
        "authenticated".to_owned(),
        json!(settings.api_key.is_some()),
    );
    fields.insert("batch_size".to_owned(), json!(settings.batch.batch_size));
    fields.insert(
        "flush_interval".to_owned(),
        json!(settings.batch.flush_interval.as_secs()),
    );
    Ok(scenario)
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        run_scenario(input)
    }
}
