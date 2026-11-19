#![cfg(test)]

use rstest::rstest;

use super::{EndpointType, parse_model};
use crate::ragflow_chat::error::RagflowError;

#[rstest]
#[case::chat("ragflow/chat/abc/gpt-4o", EndpointType::Chat, "abc", "gpt-4o")]
#[case::agent("ragflow/agent/def/glm-4", EndpointType::Agent, "def", "glm-4")]
#[case::model_with_slashes(
    "ragflow/chat/abc/openai/gpt-4o",
    EndpointType::Chat,
    "abc",
    "openai/gpt-4o"
)]
#[case::empty_model_name("ragflow/agent/abc/", EndpointType::Agent, "abc", "")]
#[case::empty_entity_id("ragflow/chat//m", EndpointType::Chat, "", "m")]
#[case::leading_slash_in_model("ragflow/chat/abc//m", EndpointType::Chat, "abc", "/m")]
fn models_parse_into_the_endpoint_id_and_model(
    #[case] wire: &str,
    #[case] endpoint: EndpointType,
    #[case] entity_id: &str,
    #[case] model: &str,
) {
    let parsed = parse_model(wire).expect("the fixture model parses");
    assert_eq!(parsed.endpoint, endpoint);
    assert_eq!(parsed.entity_id, entity_id);
    assert_eq!(parsed.model, model);
}

#[rstest]
#[case::three_parts("ragflow/chat/abc")]
#[case::two_parts("ragflow/chat")]
#[case::provider_only("ragflow")]
#[case::empty("")]
fn short_models_report_the_expected_format(#[case] wire: &str) {
    let error = parse_model(wire).expect_err("a short model is refused");
    assert!(matches!(error, RagflowError::ModelTooShort { .. }));
    let expected = [
        "Invalid RAGFlow model format: ",
        wire,
        ". Expected format: ragflow/chat/{chat_id}/{model} or ragflow/agent/{agent_id}/{model}",
    ]
    .concat();
    assert_eq!(error.to_string(), expected);
}

#[test]
fn a_short_model_with_another_prefix_is_still_too_short() {
    let error = parse_model("other/chat/abc").expect_err("a short model is refused");
    assert!(matches!(error, RagflowError::ModelTooShort { .. }));
    assert_eq!(
        error.to_string(),
        "Invalid RAGFlow model format: other/chat/abc. Expected format: ragflow/chat/{chat_id}/{model} or ragflow/agent/{agent_id}/{model}"
    );
}

#[test]
fn a_wrong_first_part_reports_the_prefix_message() {
    let error = parse_model("other/chat/abc/m").expect_err("a wrong prefix is refused");
    assert!(matches!(error, RagflowError::ModelWrongPrefix { .. }));
    assert_eq!(
        error.to_string(),
        "Invalid RAGFlow model format: other/chat/abc/m. Must start with 'ragflow/'"
    );
}

#[rstest]
#[case::unknown_type("ragflow/completion/abc/m", "completion")]
#[case::capitalized("ragflow/Chat/abc/m", "Chat")]
#[case::empty_type("ragflow//abc/m", "")]
fn an_unknown_endpoint_type_reports_the_allowed_values(#[case] wire: &str, #[case] kind: &str) {
    let error = parse_model(wire).expect_err("an unknown endpoint type is refused");
    assert!(matches!(error, RagflowError::EndpointType { .. }));
    assert_eq!(
        error.to_string(),
        [
            "Invalid RAGFlow endpoint type: ",
            kind,
            ". Must be 'chat' or 'agent'"
        ]
        .concat()
    );
}
