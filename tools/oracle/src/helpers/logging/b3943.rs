use std::time::Duration;

use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::newrelic_metrics::aggregation::build_metric_payload;
use betterlite_logging::newrelic_metrics::records::{
    NewRelicMetricRecord, metric_record_from_payload,
};
use betterlite_logging::newrelic_metrics::region::resolve_metric_endpoint;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Region {
        #[serde(default)]
        region: Option<String>,
    },
    Record {
        payload: Box<StandardLoggingPayload>,
    },
    Payload {
        records: Vec<StandardLoggingPayload>,
        window_start_ms: u64,
        now_ms: u64,
    },
}

#[derive(Serialize)]
struct RegionOutput {
    endpoint: &'static str,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Region { region } => serde_json::to_value(RegionOutput {
                endpoint: resolve_metric_endpoint(region.as_deref()),
            })
            .map_err(value_error),
            Input::Record { payload } => {
                let record = metric_record_from_payload(payload).map_err(value_error)?;
                serde_json::to_value(record).map_err(value_error)
            }
            Input::Payload {
                records,
                window_start_ms,
                now_ms,
            } => payload_output(records, *window_start_ms, *now_ms),
        }
    }
}

fn payload_output(
    records: &[StandardLoggingPayload],
    window_start_ms: u64,
    now_ms: u64,
) -> crate::helpers::HelperResult {
    let extracted = records
        .iter()
        .map(metric_record_from_payload)
        .collect::<Result<Vec<NewRelicMetricRecord>, _>>()
        .map_err(value_error)?;
    let envelope = build_metric_payload(
        &extracted,
        Duration::from_millis(window_start_ms),
        Duration::from_millis(now_ms),
    )
    .map_err(value_error)?;
    serde_json::to_value(vec![envelope]).map_err(value_error)
}

fn value_error<E>(error: E) -> Box<crate::helpers::HelperError>
where
    E: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
