#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CachePointSupport {
    #[default]
    Unsupported,
    ShortTtl,
    ExtendedTtl,
}

// The caller resolves these from the model it routes to; LiteLLM reads the same facts from its price map.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BedrockToolCapabilities {
    pub cache_points: CachePointSupport,
    pub supports_strict_tools: bool,
}
