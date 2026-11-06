const STOP: [&str; 16] = [
    "stop",
    "stop_sequence",
    "end_turn",
    "COMPLETE",
    "ERROR",
    "eos_token",
    "eos",
    "STOP",
    "FINISH_REASON_UNSPECIFIED",
    "MALFORMED_FUNCTION_CALL",
    "TOO_MANY_TOOL_CALLS",
    "MALFORMED_RESPONSE",
    "UNEXPECTED_TOOL_CALL",
    "MISSING_THOUGHT_SIGNATURE",
    "network_error",
    "error",
];
const LENGTH: [&str; 4] = ["length", "max_tokens", "compaction", "MAX_TOKENS"];
const TOOL_CALLS: [&str; 2] = ["tool_calls", "tool_use"];
const CONTENT_FILTER: [&str; 19] = [
    "content_filter",
    "refusal",
    "ERROR_TOXIC",
    "SAFETY",
    "RECITATION",
    "LANGUAGE",
    "OTHER",
    "BLOCKLIST",
    "PROHIBITED_CONTENT",
    "SPII",
    "IMAGE_SAFETY",
    "IMAGE_PROHIBITED_CONTENT",
    "NO_IMAGE",
    "IMAGE_RECITATION",
    "IMAGE_OTHER",
    "ESCALATION",
    "sensitive",
    "guardrail_intervened",
    "content_filtered",
];

// LiteLLM maps every provider's stop vocabulary onto the OpenAI one before an event is logged
pub(super) fn map_finish_reason(reason: &str) -> &'static str {
    if CONTENT_FILTER.contains(&reason) {
        return "content_filter";
    }
    if STOP.contains(&reason) {
        return "stop";
    }
    if LENGTH.contains(&reason) {
        return "length";
    }
    if TOOL_CALLS.contains(&reason) {
        return "tool_calls";
    }
    if reason == "function_call" {
        return "function_call";
    }
    tracing::warn!(finish_reason = reason, "unmapped finish_reason; using stop");
    "stop"
}
