#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    FunctionCall,
    ContentFilter,
}

impl FinishReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Length => "length",
            Self::ToolCalls => "tool_calls",
            Self::FunctionCall => "function_call",
            Self::ContentFilter => "content_filter",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinishReasonMapping {
    pub reason: FinishReason,
    pub recognized: bool,
}

// LiteLLM's _FINISH_REASON_MAP, one entry per raw reason it recognizes
const FINISH_REASONS: [(&str, FinishReason); 42] = [
    ("stop_sequence", FinishReason::Stop),
    ("end_turn", FinishReason::Stop),
    ("COMPLETE", FinishReason::Stop),
    ("ERROR", FinishReason::Stop),
    ("eos_token", FinishReason::Stop),
    ("eos", FinishReason::Stop),
    ("STOP", FinishReason::Stop),
    ("FINISH_REASON_UNSPECIFIED", FinishReason::Stop),
    ("MALFORMED_FUNCTION_CALL", FinishReason::Stop),
    ("TOO_MANY_TOOL_CALLS", FinishReason::Stop),
    ("MALFORMED_RESPONSE", FinishReason::Stop),
    ("UNEXPECTED_TOOL_CALL", FinishReason::Stop),
    ("MISSING_THOUGHT_SIGNATURE", FinishReason::Stop),
    ("network_error", FinishReason::Stop),
    ("stop", FinishReason::Stop),
    ("error", FinishReason::Stop),
    ("max_tokens", FinishReason::Length),
    ("compaction", FinishReason::Length),
    ("MAX_TOKENS", FinishReason::Length),
    ("length", FinishReason::Length),
    ("tool_use", FinishReason::ToolCalls),
    ("tool_calls", FinishReason::ToolCalls),
    ("function_call", FinishReason::FunctionCall),
    ("refusal", FinishReason::ContentFilter),
    ("ERROR_TOXIC", FinishReason::ContentFilter),
    ("SAFETY", FinishReason::ContentFilter),
    ("RECITATION", FinishReason::ContentFilter),
    ("LANGUAGE", FinishReason::ContentFilter),
    ("OTHER", FinishReason::ContentFilter),
    ("BLOCKLIST", FinishReason::ContentFilter),
    ("PROHIBITED_CONTENT", FinishReason::ContentFilter),
    ("SPII", FinishReason::ContentFilter),
    ("IMAGE_SAFETY", FinishReason::ContentFilter),
    ("IMAGE_PROHIBITED_CONTENT", FinishReason::ContentFilter),
    ("NO_IMAGE", FinishReason::ContentFilter),
    ("IMAGE_RECITATION", FinishReason::ContentFilter),
    ("IMAGE_OTHER", FinishReason::ContentFilter),
    ("ESCALATION", FinishReason::ContentFilter),
    ("sensitive", FinishReason::ContentFilter),
    ("guardrail_intervened", FinishReason::ContentFilter),
    ("content_filter", FinishReason::ContentFilter),
    ("content_filtered", FinishReason::ContentFilter),
];

#[must_use]
pub fn map_finish_reason(raw: &str) -> FinishReasonMapping {
    FINISH_REASONS
        .iter()
        .find(|(name, _reason)| *name == raw)
        .map_or(
            FinishReasonMapping {
                reason: FinishReason::Stop,
                recognized: false,
            },
            |(_name, reason)| FinishReasonMapping {
                reason: *reason,
                recognized: true,
            },
        )
}
