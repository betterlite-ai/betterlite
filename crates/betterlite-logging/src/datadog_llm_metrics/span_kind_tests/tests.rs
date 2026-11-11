#![cfg(test)]

use rstest::rstest;

use crate::datadog_llm_metrics::span_kind::span_kind;
use crate::datadog_llm_obs::span_records::SpanKind;

const PARENT: &str = "parent-span";

#[rstest]
#[case::completion("completion")]
#[case::acompletion("acompletion")]
#[case::text_completion("text_completion")]
#[case::atext_completion("atext_completion")]
#[case::generate_content("generate_content")]
#[case::agenerate_content("agenerate_content")]
#[case::generate_content_stream("generate_content_stream")]
#[case::agenerate_content_stream("agenerate_content_stream")]
#[case::anthropic_messages("anthropic_messages")]
#[case::responses("responses")]
#[case::aresponses("aresponses")]
fn completion_call_types_are_llm(#[case] call_type: &str) {
    assert_eq!(span_kind(Some(call_type), Some(PARENT)), SpanKind::Llm);
}

#[rstest]
#[case::embedding("embedding")]
#[case::aembedding("aembedding")]
fn embedding_call_types_are_embedding(#[case] call_type: &str) {
    assert_eq!(
        span_kind(Some(call_type), Some(PARENT)),
        SpanKind::Embedding
    );
}

#[test]
fn mcp_tool_calls_are_tool() {
    assert_eq!(
        span_kind(Some("call_mcp_tool"), Some(PARENT)),
        SpanKind::Tool
    );
}

#[rstest]
#[case::get_assistants("get_assistants")]
#[case::aget_assistants("aget_assistants")]
#[case::get_thread("get_thread")]
#[case::aget_thread("aget_thread")]
#[case::get_messages("get_messages")]
#[case::aget_messages("aget_messages")]
#[case::afile_retrieve("afile_retrieve")]
#[case::file_retrieve("file_retrieve")]
#[case::afile_list("afile_list")]
#[case::file_list("file_list")]
#[case::afile_content("afile_content")]
#[case::file_content("file_content")]
#[case::retrieve_batch("retrieve_batch")]
#[case::aretrieve_batch("aretrieve_batch")]
#[case::retrieve_fine_tuning_job("retrieve_fine_tuning_job")]
#[case::aretrieve_fine_tuning_job("aretrieve_fine_tuning_job")]
#[case::alist_input_items("alist_input_items")]
fn retrieval_call_types_are_retrieval(#[case] call_type: &str) {
    assert_eq!(
        span_kind(Some(call_type), Some(PARENT)),
        SpanKind::Retrieval
    );
}

#[rstest]
#[case::create_batch("create_batch")]
#[case::acreate_batch("acreate_batch")]
#[case::create_fine_tuning_job("create_fine_tuning_job")]
#[case::acreate_fine_tuning_job("acreate_fine_tuning_job")]
#[case::cancel_fine_tuning_job("cancel_fine_tuning_job")]
#[case::acancel_fine_tuning_job("acancel_fine_tuning_job")]
#[case::list_fine_tuning_jobs("list_fine_tuning_jobs")]
#[case::alist_fine_tuning_jobs("alist_fine_tuning_jobs")]
#[case::create_assistants("create_assistants")]
#[case::acreate_assistants("acreate_assistants")]
#[case::delete_assistant("delete_assistant")]
#[case::adelete_assistant("adelete_assistant")]
#[case::create_thread("create_thread")]
#[case::acreate_thread("acreate_thread")]
#[case::add_message("add_message")]
#[case::a_add_message("a_add_message")]
#[case::run_thread("run_thread")]
#[case::arun_thread("arun_thread")]
#[case::run_thread_stream("run_thread_stream")]
#[case::arun_thread_stream("arun_thread_stream")]
#[case::file_delete("file_delete")]
#[case::afile_delete("afile_delete")]
#[case::create_file("create_file")]
#[case::acreate_file("acreate_file")]
#[case::image_generation("image_generation")]
#[case::aimage_generation("aimage_generation")]
#[case::image_edit("image_edit")]
#[case::aimage_edit("aimage_edit")]
#[case::moderation("moderation")]
#[case::amoderation("amoderation")]
#[case::transcription("transcription")]
#[case::atranscription("atranscription")]
#[case::speech("speech")]
#[case::aspeech("aspeech")]
#[case::rerank("rerank")]
#[case::arerank("arerank")]
fn task_call_types_are_task(#[case] call_type: &str) {
    assert_eq!(span_kind(Some(call_type), Some(PARENT)), SpanKind::Task);
}

#[rstest]
#[case::embedding("embedding")]
#[case::tool("call_mcp_tool")]
#[case::retrieval("get_assistants")]
#[case::task("rerank")]
fn a_missing_parent_falls_back_to_llm(#[case] call_type: &str) {
    assert_eq!(span_kind(Some(call_type), None), SpanKind::Llm);
}

#[test]
fn a_missing_call_type_is_llm() {
    assert_eq!(span_kind(None, Some(PARENT)), SpanKind::Llm);
}

#[test]
fn an_empty_call_type_is_llm() {
    assert_eq!(span_kind(Some(""), Some(PARENT)), SpanKind::Llm);
}

#[test]
fn an_empty_parent_id_still_counts_as_a_parent() {
    assert_eq!(span_kind(Some("embedding"), Some("")), SpanKind::Embedding);
}

#[test]
fn an_unknown_call_type_is_llm() {
    assert_eq!(span_kind(Some("a_new_call"), Some(PARENT)), SpanKind::Llm);
}

#[test]
fn listing_mcp_tools_is_not_a_tool_span() {
    assert_eq!(
        span_kind(Some("list_mcp_tools"), Some(PARENT)),
        SpanKind::Llm
    );
}
