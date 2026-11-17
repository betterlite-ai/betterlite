// LiteLLM matches these three classic Claude2 names exactly and gives every other anthropic model the newer config.
pub const LEGACY_ANTHROPIC_MODEL_NAMES: [&str; 3] = [
    "anthropic.claude-v2",
    "anthropic.claude-instant-v1",
    "anthropic.claude-v2:1",
];
