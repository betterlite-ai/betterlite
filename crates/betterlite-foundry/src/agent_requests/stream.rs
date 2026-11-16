pub const SHOULD_FAKE_STREAM: bool = true;
pub const HAS_CUSTOM_STREAM_WRAPPER: bool = false;
pub const SUPPORTS_STREAM_PARAM_IN_BODY: bool = false;

// Azure Agents polls for the run result, so LiteLLM fakes the stream and the custom handler owns response conversion (B561)
