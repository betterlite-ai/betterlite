use std::time::Duration;

use betterlite_logging::logging_streams::duration::record_call_duration;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use chrono::{DateTime, FixedOffset, Utc};

use crate::image_call::plan::LogContext;

const ASYNC_CALL_TYPE: &str = "get_async_custom_stream_wrapper <- track_llm_api_timing";
const SYNC_CALL_TYPE: &str = "get_sync_custom_stream_wrapper <- track_llm_api_timing";

// ServiceTypes.LITELLM's success hook; the call layer implements it until betterlite ports services.
pub trait ApiTiming: Send + Sync {
    fn service_success(&self, call: &TimingCall<'_>);
}

pub struct TimingCall<'call> {
    pub call_type: &'call str,
    pub duration: f64,
    pub start_time: f64,
    pub end_time: f64,
}

// datetime.now() at the same reading time.time() gives: whole seconds plus the fraction.
fn datetime(instant: &StreamInstant) -> Option<DateTime<FixedOffset>> {
    let elapsed = Duration::try_from_secs_f64(instant.timestamp).ok()?;
    let seconds = i64::try_from(elapsed.as_secs()).ok()?;
    DateTime::<Utc>::from_timestamp(seconds, elapsed.subsec_nanos()).map(DateTime::fixed_offset)
}

// track_llm_api_timing's finally block: the duration on model_call_details, then the service hook.
pub(super) fn record(
    log: &mut LogContext<'_, '_>,
    timing: &dyn ApiTiming,
    sync: bool,
    started: &StreamInstant,
    ended: &StreamInstant,
) {
    let begin = datetime(started);
    let finish = datetime(ended);
    if let (Some(begin), Some(finish)) = (begin, finish)
        && let Err(error) = record_call_duration(log.state, begin, finish)
    {
        // _set_duration_in_model_call_details catches its own errors, and the service call still follows.
        tracing::warn!(%error, "Error setting `llm_api_duration_ms`");
    }
    timing.service_success(&TimingCall {
        call_type: if sync {
            SYNC_CALL_TYPE
        } else {
            ASYNC_CALL_TYPE
        },
        duration: started.timestamp.mul_add(-1.0, ended.timestamp),
        start_time: started.timestamp,
        end_time: ended.timestamp,
    });
}
