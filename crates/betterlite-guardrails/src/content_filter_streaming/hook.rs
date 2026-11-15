use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use indexmap::IndexMap;
use tokio::sync::mpsc;

use crate::content_filter::risk_scoring::GuardrailStatus;
use crate::content_filter_actions::ContentFilterSource;
use crate::content_filter_streaming::error::StreamFilterError;
use crate::content_filter_streaming::logging::log_stream_record;
use crate::content_filter_streaming::plan::{StreamedScanPlan, streamed_scan_plan};
use crate::content_filter_streaming::state::StreamedChoiceState;
use crate::guardrail_base::inputs::GuardrailLogTarget;

mod scan;
mod tests;

// The guardrail settings the streaming hook reads; LiteLLM keeps them on the guardrail object
#[derive(Debug, Clone)]
pub struct StreamFilterSettings {
    pub guardrail_name: String,
    pub event_hook: LitellmParamsMode,
    pub guardrail_id: Option<String>,
    pub policy_template: Option<String>,
    pub pattern_redaction_format: String,
    pub keyword_redaction_tag: String,
}

// LiteLLM's hook arguments: the caller's key object is accepted and the hook never reads it
pub struct StreamFilterCall<'call> {
    pub user_api_key_dict: Option<&'call GuardrailValue>,
    pub request_data: &'call mut BTreeMap<String, GuardrailValue>,
    pub log_target: Option<&'call mut GuardrailLogTarget>,
}

// Everything a chunk needs besides the chunk itself; the hook keeps it for one stream
pub(super) struct StreamScan<'scan, 'source> {
    pub(super) source: &'scan ContentFilterSource<'source>,
    pub(super) plan: StreamedScanPlan,
    pub(super) states: IndexMap<i64, StreamedChoiceState>,
    pub(super) scan_time: Duration,
}

pub async fn stream_filter_hook(
    mut response: mpsc::Receiver<GuardrailValue>,
    output: mpsc::Sender<GuardrailValue>,
    settings: &StreamFilterSettings,
    source: &ContentFilterSource<'_>,
    call: &mut StreamFilterCall<'_>,
) -> Result<(), StreamFilterError> {
    let started = SystemTime::now();
    let mut scan = StreamScan {
        source,
        plan: streamed_scan_plan(source),
        states: IndexMap::new(),
        scan_time: Duration::ZERO,
    };
    let outcome = drive_stream(&mut response, &output, &mut scan).await;
    let status = stream_status(&outcome);
    log_stream_record(settings, &scan, call, status, started)?;
    outcome
}

#[must_use]
pub const fn stream_status(outcome: &Result<(), StreamFilterError>) -> GuardrailStatus {
    match outcome {
        Ok(()) => GuardrailStatus::Success,
        Err(StreamFilterError::Blocked { .. }) => GuardrailStatus::GuardrailIntervened,
        Err(StreamFilterError::Record(_) | StreamFilterError::Base(_)) => {
            GuardrailStatus::GuardrailFailedToRespond
        }
    }
}

async fn drive_stream(
    response: &mut mpsc::Receiver<GuardrailValue>,
    output: &mpsc::Sender<GuardrailValue>,
    scan: &mut StreamScan<'_, '_>,
) -> Result<(), StreamFilterError> {
    while let Some(mut item) = response.recv().await {
        // a stopped consumer closed the channel; its suspended yield never scans the next chunk
        let Ok(permit) = output.reserve().await else {
            break;
        };
        scan::process_chunk(&mut item, scan)?;
        permit.send(item);
    }
    Ok(())
}
