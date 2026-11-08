pub(crate) const OBSERVATION_TYPE: &str = "langfuse.observation.type";

// litellm.types.integrations.langfuse_otel.LangfuseSpanAttributes
#[derive(Clone, Copy, Debug)]
pub enum LangfuseSpanAttributes {
    LangfuseEnvironment,
    Version,
    Release,
    GenerationName,
    GenerationId,
    ParentObservationId,
    MaskInput,
    MaskOutput,
    ObservationInput,
    ObservationOutput,
    TraceUserId,
    SessionId,
    Tags,
    TraceName,
    TraceId,
    TraceMetadata,
    ExistingTraceId,
    UpdateTraceKeys,
    DebugLangfuse,
}

impl LangfuseSpanAttributes {
    #[must_use]
    pub const fn value(self) -> &'static str {
        match self {
            Self::LangfuseEnvironment => "langfuse.environment",
            Self::Version => "langfuse.version",
            Self::Release => "langfuse.release",
            Self::GenerationName => "langfuse.generation.name",
            Self::GenerationId => "langfuse.generation.id",
            Self::ParentObservationId => "langfuse.generation.parent_observation_id",
            Self::MaskInput => "langfuse.generation.mask_input",
            Self::MaskOutput => "langfuse.generation.mask_output",
            Self::ObservationInput => "langfuse.observation.input",
            Self::ObservationOutput => "langfuse.observation.output",
            Self::TraceUserId => "user.id",
            Self::SessionId => "session.id",
            Self::Tags => "langfuse.trace.tags",
            Self::TraceName => "langfuse.trace.name",
            Self::TraceId => "langfuse.trace.id",
            Self::TraceMetadata => "langfuse.trace.metadata",
            Self::ExistingTraceId => "langfuse.trace.existing_id",
            Self::UpdateTraceKeys => "langfuse.trace.update_keys",
            Self::DebugLangfuse => "langfuse.debug",
        }
    }
}
