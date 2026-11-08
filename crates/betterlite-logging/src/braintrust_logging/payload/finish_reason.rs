pub(super) fn map_finish_reason(reason: &str) -> &'static str {
    if stops(reason) {
        return "stop";
    }
    // LiteLLM maps every provider's stop vocabulary onto the OpenAI one before an event is logged
    match reason {
        "length" | "max_tokens" | "compaction" | "MAX_TOKENS" => "length",
        "tool_calls" | "tool_use" => "tool_calls",
        "function_call" => "function_call",
        "content_filter"
        | "refusal"
        | "ERROR_TOXIC"
        | "SAFETY"
        | "RECITATION"
        | "LANGUAGE"
        | "OTHER"
        | "BLOCKLIST"
        | "PROHIBITED_CONTENT"
        | "SPII"
        | "IMAGE_SAFETY"
        | "IMAGE_PROHIBITED_CONTENT"
        | "NO_IMAGE"
        | "IMAGE_RECITATION"
        | "IMAGE_OTHER"
        | "ESCALATION"
        | "sensitive"
        | "guardrail_intervened"
        | "content_filtered" => "content_filter",
        other => {
            tracing::warn!(finish_reason = other, "unmapped finish_reason; using stop");
            "stop"
        }
    }
}

fn stops(reason: &str) -> bool {
    matches!(
        reason,
        "stop"
            | "stop_sequence"
            | "end_turn"
            | "COMPLETE"
            | "ERROR"
            | "eos_token"
            | "eos"
            | "STOP"
            | "FINISH_REASON_UNSPECIFIED"
            | "MALFORMED_FUNCTION_CALL"
            | "TOO_MANY_TOOL_CALLS"
            | "MALFORMED_RESPONSE"
            | "UNEXPECTED_TOOL_CALL"
            | "MISSING_THOUGHT_SIGNATURE"
            | "network_error"
            | "error"
    )
}
