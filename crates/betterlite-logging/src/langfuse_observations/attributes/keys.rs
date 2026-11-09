pub(crate) const TRACE_NAME: &str = "langfuse.trace.name";
pub(crate) const TRACE_USER_ID: &str = "user.id";
pub(crate) const TRACE_SESSION_ID: &str = "session.id";
pub(crate) const TRACE_TAGS: &str = "langfuse.trace.tags";
pub(crate) const TRACE_PUBLIC: &str = "langfuse.trace.public";
pub(crate) const TRACE_METADATA: &str = "langfuse.trace.metadata";
pub(crate) const TRACE_INPUT: &str = "langfuse.trace.input";
pub(crate) const TRACE_OUTPUT: &str = "langfuse.trace.output";

// langfuse._client.attributes.LangfuseOtelSpanAttributes, which the v4 ingestion reads instead of v2's body
pub(crate) const OBSERVATION_TYPE: &str = "langfuse.observation.type";
pub(crate) const OBSERVATION_METADATA: &str = "langfuse.observation.metadata";
pub(crate) const OBSERVATION_LEVEL: &str = "langfuse.observation.level";
pub(crate) const OBSERVATION_STATUS_MESSAGE: &str = "langfuse.observation.status_message";
pub(crate) const OBSERVATION_INPUT: &str = "langfuse.observation.input";
pub(crate) const OBSERVATION_OUTPUT: &str = "langfuse.observation.output";
pub(crate) const COMPLETION_START: &str = "langfuse.observation.completion_start_time";
pub(crate) const OBSERVATION_MODEL: &str = "langfuse.observation.model.name";
pub(crate) const OBSERVATION_MODEL_PARAMETERS: &str = "langfuse.observation.model.parameters";
pub(crate) const OBSERVATION_USAGE_DETAILS: &str = "langfuse.observation.usage_details";
pub(crate) const OBSERVATION_COST_DETAILS: &str = "langfuse.observation.cost_details";
pub(crate) const OBSERVATION_PROMPT_NAME: &str = "langfuse.observation.prompt.name";
pub(crate) const OBSERVATION_PROMPT_VERSION: &str = "langfuse.observation.prompt.version";

pub(crate) const RELEASE: &str = "langfuse.release";
pub(crate) const VERSION: &str = "langfuse.version";
