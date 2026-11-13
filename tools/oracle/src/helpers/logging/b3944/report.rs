use std::collections::BTreeMap;
use std::io::Read as _;

use betterlite_logging::newrelic_metrics::logger::NewRelicMetricsLogger;
use betterlite_logging::newrelic_metrics::queue::queue_snapshot;
use flate2::read::GzDecoder;
use serde_json::{Value, json};

use super::scripted::{RecordedRequest, ScriptedSleeps, ScriptedTransport};
use super::{HelperResult, failure};

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];
const QUEUE_PREVIEW: usize = 3;

pub(super) fn scenario(
    logger: &NewRelicMetricsLogger,
    transport: &ScriptedTransport,
    sleeps: &ScriptedSleeps,
    scheduled: Option<bool>,
    delivered: Option<bool>,
) -> HelperResult {
    let recorded = transport.requests();
    let requests = Value::Array(recorded.iter().map(request_value).collect());
    Ok(json!({
        "requests": requests,
        "queue": queue_value(logger)?,
        "sleeps": sleeps.seconds(),
        "scheduled": scheduled,
        "delivered": delivered,
    }))
}

fn queue_value(logger: &NewRelicMetricsLogger) -> HelperResult {
    let queue = queue_snapshot(logger);
    let head = queue
        .iter()
        .take(QUEUE_PREVIEW)
        .map(|record| serde_json::to_value(record).map_err(failure))
        .collect::<Result<Vec<Value>, _>>()?;
    let tail = queue
        .iter()
        .rev()
        .take(QUEUE_PREVIEW)
        .rev()
        .map(|record| serde_json::to_value(record).map_err(failure))
        .collect::<Result<Vec<Value>, _>>()?;
    Ok(json!({
        "length": queue.len(),
        "head": head,
        "tail": tail,
    }))
}

fn request_value(request: &RecordedRequest) -> Value {
    let gzipped = request.body.starts_with(&GZIP_MAGIC);
    let raw = if gzipped {
        gunzip(&request.body)
    } else {
        request.body.clone()
    };
    let body: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
    let headers: BTreeMap<String, String> = request
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_ascii_lowercase(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    json!({
        "url": request.url,
        "headers": headers,
        "body": body,
        "gzip": gzipped,
    })
}

fn gunzip(body: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    if GzDecoder::new(body).read_to_end(&mut decoded).is_err() {
        return body.to_vec();
    }
    decoded
}
