#![cfg(test)]

use rstest::rstest;

use crate::otel::vocabulary::constants::{GenAIOperation, GenAIOutputType};
use crate::otel::vocabulary::registry::SpanRole;
use crate::otel::vocabulary::resolvers::{
    db_system, resolve_operation, resolve_output_type, resolve_provider, span_role_for_service,
};

#[rstest]
#[case::lowercased("BEDROCK", "aws.bedrock")]
#[case::text_completion_openai("text-completion-openai", "openai")]
#[case::unmapped_passes_through("Vendor_New", "Vendor_New")]
#[case::empty("", "")]
fn providers_resolve_to_semconv_names(#[case] provider: &str, #[case] expected: &str) {
    assert_eq!(resolve_provider(provider), expected);
}

#[rstest]
#[case::empty("", GenAIOperation::Chat)]
#[case::responses("aresponses", GenAIOperation::Chat)]
#[case::uppercase_embedding("AEMBEDDING", GenAIOperation::Embeddings)]
#[case::agent("send_message", GenAIOperation::InvokeAgent)]
#[case::file_management(
    "avector_store_file_delete",
    GenAIOperation::LitellmVectorStoreFileManagement
)]
#[case::unmapped_falls_back("new_route", GenAIOperation::Chat)]
fn call_types_resolve_to_operations(#[case] call_type: &str, #[case] expected: GenAIOperation) {
    assert_eq!(resolve_operation(call_type), expected);
}

#[rstest]
#[case::image("aimage_generation", Some(GenAIOutputType::Image))]
#[case::ocr("OCR", Some(GenAIOutputType::Text))]
#[case::chat("completion", None)]
fn only_pinned_routes_have_an_output_type(
    #[case] call_type: &str,
    #[case] expected: Option<GenAIOutputType>,
) {
    assert_eq!(resolve_output_type(call_type), expected);
}

#[rstest]
#[case::spend_queue("redis_spend_update_queue", Some("redis"), Some(SpanRole::DbCall))]
#[case::batch_write("batch_write_to_db", Some("postgresql"), Some(SpanRole::DbCall))]
#[case::prefix_is_case_sensitive("Redis", None, Some(SpanRole::Service))]
#[case::metrics_only("auth", None, None)]
fn services_classify_as_datastores_or_internal_work(
    #[case] service_name: &str,
    #[case] system: Option<&str>,
    #[case] role: Option<SpanRole>,
) {
    assert_eq!(db_system(service_name), system);
    assert_eq!(span_role_for_service(service_name), role);
}
