use crate::capabilities::capability_struct;
use serde::{Deserialize, Serialize};

capability_struct! {
    #[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    pub struct ReasoningCapabilities {
        supports_reasoning: "supports_reasoning",
        supports_legacy_thinking: "supports_legacy_thinking",
        supports_adaptive_thinking: "supports_adaptive_thinking",
        supports_anthropic_thinking: "supports_anthropic_thinking_payload",
        supports_anthropic_compaction: "supports_anthropic_compaction",
        supports_none_reasoning_effort: "supports_none_reasoning_effort",
        supports_min_reasoning_effort: "supports_minimal_reasoning_effort",
        supports_low_reasoning_effort: "supports_low_reasoning_effort",
        supports_max_reasoning_effort: "supports_max_reasoning_effort",
        supports_xhigh_reason_effort: "supports_xhigh_reasoning_effort",
        supports_thinking_preserve: "supports_thinking_cache_preservation",
        thinking_always_on: "thinking_always_on",
    }
}
