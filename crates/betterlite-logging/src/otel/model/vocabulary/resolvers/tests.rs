#![cfg(test)]

use crate::otel::vocabulary::*;

#[test]
fn provider_operation_and_output_mapping() {
    assert_eq!(resolve_provider(Some("BEDROCK")), "aws.bedrock");
    assert_eq!(resolve_provider(Some("new_provider")), "new_provider");
    assert_eq!(resolve_provider(None), "");
    assert_eq!(
        resolve_operation(Some("aembedding")),
        GenAIOperation::EMBEDDINGS
    );
    assert_eq!(resolve_operation(Some("new_route")), GenAIOperation::CHAT);
    assert_eq!(
        resolve_output_type(Some("image_generation")),
        Some(GenAIOutputType::IMAGE)
    );
    assert_eq!(resolve_output_type(Some("completion")), None);
}
