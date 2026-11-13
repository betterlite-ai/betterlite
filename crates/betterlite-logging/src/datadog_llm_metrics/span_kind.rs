use crate::datadog_llm_obs::span_records::SpanKind;

const RETRIEVAL_CALL_TYPES: [&str; 17] = [
    "get_assistants",
    "aget_assistants",
    "get_thread",
    "aget_thread",
    "get_messages",
    "aget_messages",
    "afile_retrieve",
    "file_retrieve",
    "afile_list",
    "file_list",
    "afile_content",
    "file_content",
    "retrieve_batch",
    "aretrieve_batch",
    "retrieve_fine_tuning_job",
    "aretrieve_fine_tuning_job",
    "alist_input_items",
];

const TASK_CALL_TYPES: [&str; 36] = [
    "create_batch",
    "acreate_batch",
    "create_fine_tuning_job",
    "acreate_fine_tuning_job",
    "cancel_fine_tuning_job",
    "acancel_fine_tuning_job",
    "list_fine_tuning_jobs",
    "alist_fine_tuning_jobs",
    "create_assistants",
    "acreate_assistants",
    "delete_assistant",
    "adelete_assistant",
    "create_thread",
    "acreate_thread",
    "add_message",
    "a_add_message",
    "run_thread",
    "arun_thread",
    "run_thread_stream",
    "arun_thread_stream",
    "file_delete",
    "afile_delete",
    "create_file",
    "acreate_file",
    "image_generation",
    "aimage_generation",
    "image_edit",
    "aimage_edit",
    "moderation",
    "amoderation",
    "transcription",
    "atranscription",
    "speech",
    "aspeech",
    "rerank",
    "arerank",
];

// Only llm and task spans can stand alone, so a span without a parent is always llm
#[must_use]
pub fn span_kind(call_type: Option<&str>, parent_id: Option<&str>) -> SpanKind {
    let (Some(name), Some(_parent)) = (call_type, parent_id) else {
        return SpanKind::Llm;
    };
    match name {
        "embedding" | "aembedding" => SpanKind::Embedding,
        "call_mcp_tool" => SpanKind::Tool,
        known if RETRIEVAL_CALL_TYPES.contains(&known) => SpanKind::Retrieval,
        known if TASK_CALL_TYPES.contains(&known) => SpanKind::Task,
        _ => SpanKind::Llm,
    }
}
