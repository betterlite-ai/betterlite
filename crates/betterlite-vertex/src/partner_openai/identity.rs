// VertexAIMistralConfig is the only partner transformation that changes its provider identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartnerClass {
    Ai21,
    GptOss,
    Llama3,
    Mistral,
}

impl PartnerClass {
    #[must_use]
    pub const fn custom_llm_provider(self) -> Option<&'static str> {
        match self {
            Self::Mistral => Some("vertex_ai"),
            Self::Ai21 | Self::GptOss | Self::Llama3 => None,
        }
    }
}
