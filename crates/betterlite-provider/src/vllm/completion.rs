use super::error::{ENGINE_MISSING_MESSAGE, ENGINE_STATUS, VllmError};

// handler.py's completion and batch_completions import vllm inside the process, so these paths return a typed error here (docs/deviations/B8153.yaml)
pub const ENGINE_ERROR: VllmError = VllmError::Engine {
    status_code: ENGINE_STATUS,
    message: ENGINE_MISSING_MESSAGE,
};
