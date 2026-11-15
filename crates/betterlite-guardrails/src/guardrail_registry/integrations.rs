pub const BEDROCK: &str = "bedrock";
pub const LAKERA: &str = "lakera";
pub const LAKERA_V2: &str = "lakera_v2";
pub const PRESIDIO: &str = "presidio";
pub const HIDE_SECRETS: &str = "hide-secrets";
pub const TOOL_PERMISSION: &str = "tool_permission";
pub const TOOL_POLICY: &str = "tool_policy";
// registered by LiteLLM in the same table, but their builders belong to B3772 and B3809
pub const GRAYSWAN: &str = "grayswan";
pub const LLM_AS_A_JUDGE: &str = "llm_as_a_judge";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionKind {
    Headroom,
    Compresr,
    Typesafe,
}

#[must_use]
pub fn compression_kind(provider: &str) -> Option<CompressionKind> {
    match provider {
        "headroom" => Some(CompressionKind::Headroom),
        "compresr" => Some(CompressionKind::Compresr),
        "typesafe" => Some(CompressionKind::Typesafe),
        _not_compression => None,
    }
}
