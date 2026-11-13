pub mod keys;
pub mod llm_obs;
pub mod output_content;
pub mod writer;

mod metadata;
mod observation;
mod wire;

#[path = "langfuse_otel_attributes/tests/debug.rs"]
mod debug;
#[path = "langfuse_otel_attributes/tests/dict_output.rs"]
mod dict_output;
#[path = "langfuse_otel_attributes/tests/embedding_output.rs"]
mod embedding_output;
#[path = "langfuse_otel_attributes/tests/empty_output.rs"]
mod empty_output;
#[path = "langfuse_otel_attributes/tests/existing_trace.rs"]
mod existing_trace;
#[path = "langfuse_otel_attributes/tests/extract_metadata.rs"]
mod extract_metadata;
#[path = "langfuse_otel_attributes/tests/generation_id.rs"]
mod generation_id;
#[path = "langfuse_otel_attributes/tests/generation_name.rs"]
mod generation_name;
#[path = "langfuse_otel_attributes/tests/list_output.rs"]
mod list_output;
#[path = "langfuse_otel_attributes/tests/mask.rs"]
mod mask;
#[path = "langfuse_otel_attributes/tests/metadata_enrichment.rs"]
mod metadata_enrichment;
#[path = "langfuse_otel_attributes/tests/metadata_exception.rs"]
mod metadata_exception;
#[path = "langfuse_otel_attributes/tests/observation_items_function.rs"]
mod observation_items_function;
#[path = "langfuse_otel_attributes/tests/observation_items_message.rs"]
mod observation_items_message;
#[path = "langfuse_otel_attributes/tests/observation_items_reasoning.rs"]
mod observation_items_reasoning;
#[path = "langfuse_otel_attributes/tests/observation_output_arguments.rs"]
mod observation_output_arguments;
#[path = "langfuse_otel_attributes/tests/observation_output_message.rs"]
mod observation_output_message;
#[path = "langfuse_otel_attributes/tests/observation_output_tool_calls.rs"]
mod observation_output_tool_calls;
#[path = "langfuse_otel_attributes/tests/parent_observation.rs"]
mod parent_observation;
#[path = "langfuse_otel_attributes/tests/pydantic_output.rs"]
mod pydantic_output;
#[path = "langfuse_otel_attributes/tests/session_id.rs"]
mod session_id;
#[path = "langfuse_otel_attributes/tests/set_langfuse_otel_attributes.rs"]
mod set_langfuse_otel_attributes;
#[path = "langfuse_otel_attributes/tests/set_messages.rs"]
mod set_messages;
#[path = "langfuse_otel_attributes/tests/set_output.rs"]
mod set_output;
#[path = "langfuse_otel_attributes/tests/speech_output.rs"]
mod speech_output;
#[path = "langfuse_otel_attributes/tests/support.rs"]
mod support;
#[path = "langfuse_otel_attributes/tests/tags.rs"]
mod tags;
#[path = "langfuse_otel_attributes/tests/trace_id.rs"]
mod trace_id;
#[path = "langfuse_otel_attributes/tests/trace_metadata.rs"]
mod trace_metadata;
#[path = "langfuse_otel_attributes/tests/trace_name.rs"]
mod trace_name;
#[path = "langfuse_otel_attributes/tests/trace_release.rs"]
mod trace_release;
#[path = "langfuse_otel_attributes/tests/trace_user.rs"]
mod trace_user;
#[path = "langfuse_otel_attributes/tests/update_keys.rs"]
mod update_keys;
#[path = "langfuse_otel_attributes/tests/version.rs"]
mod version;
