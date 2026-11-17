use thiserror::Error;

use super::python_text::python_float_text;
use super::state::ResponsesStreamBaseState;

pub const STREAM_TIMEOUT_STATUS: u16 = 408;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct StreamDurationTimeout {
    pub status_code: u16,
    pub message: String,
    pub model: String,
    pub custom_llm_provider: String,
}

impl ResponsesStreamBaseState {
    // the source compares with a strict greater-than, and a nan on either side never counts as exceeded
    pub fn check_max_streaming_duration(
        &self,
        now_timestamp: f64,
    ) -> Result<(), StreamDurationTimeout> {
        let Some(limit) = self.max_streaming_duration_seconds else {
            return Ok(());
        };
        // the subtraction spelled as a fused step keeps it out of arithmetic_side_effects, and -1.0*x is exact
        let elapsed = self.created_timestamp.mul_add(-1.0_f64, now_timestamp);
        let exceeded = matches!(
            elapsed.partial_cmp(&limit),
            Some(std::cmp::Ordering::Greater)
        );
        if !exceeded {
            return Ok(());
        }
        Err(StreamDurationTimeout {
            status_code: STREAM_TIMEOUT_STATUS,
            message: format!(
                "litellm.Timeout: Stream exceeded max streaming duration of {}s (elapsed {elapsed:.1}s)",
                python_float_text(limit)
            ),
            model: self.model.clone(),
            custom_llm_provider: self.custom_llm_provider.clone().unwrap_or_default(),
        })
    }
}
