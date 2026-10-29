mod tests;

#[derive(Debug, Clone)]
pub struct StackFrame {
    pub function_name: String,
    pub module_name: String,
}

pub const BREAKER_GUARD_FRAME_NAMES: [&str; 4] = [
    "<lambda>",
    "wrapper",
    "_run_under_circuit_breaker",
    "_run_under_circuit_breaker_sync",
];

// mirrors LiteLLM's _get_call_stack_info, which reports "unknown" rather than an empty join
#[must_use]
pub fn project_call_stack(frames: &[StackFrame], guard_module: &str, num_frames: usize) -> String {
    let mut names = Vec::with_capacity(num_frames);
    for frame in frames {
        if names.len() >= num_frames {
            break;
        }
        let is_guard_frame = frame.module_name == guard_module
            && BREAKER_GUARD_FRAME_NAMES.contains(&frame.function_name.as_str());
        if is_guard_frame {
            continue;
        }
        names.push(frame.function_name.clone());
    }
    if names.is_empty() {
        "unknown".to_owned()
    } else {
        names.join(" <- ")
    }
}
