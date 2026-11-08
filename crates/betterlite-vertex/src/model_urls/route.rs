#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VertexModelRoute {
    PartnerModels,
    Gemini,
    Gemma,
    Bge,
    ModelGarden,
    NonGemini,
    OpenAi,
    AgentEngine,
}

impl VertexModelRoute {
    // LiteLLM keeps this list as VertexAIModelRoute; B1306 selects the route and this layer strips its prefix
    pub const ALL: [Self; 8] = [
        Self::PartnerModels,
        Self::Gemini,
        Self::Gemma,
        Self::Bge,
        Self::ModelGarden,
        Self::NonGemini,
        Self::OpenAi,
        Self::AgentEngine,
    ];

    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::PartnerModels => "partner_models/",
            Self::Gemini => "gemini/",
            Self::Gemma => "gemma/",
            Self::Bge => "bge/",
            Self::ModelGarden => "model_garden/",
            Self::NonGemini => "non_gemini/",
            Self::OpenAi => "openai/",
            Self::AgentEngine => "agent_engine/",
        }
    }
}
