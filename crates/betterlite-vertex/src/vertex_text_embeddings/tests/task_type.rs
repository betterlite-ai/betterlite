use rstest::rstest;

use crate::vertex_text_embeddings::task_type::TaskType;

#[rstest]
#[case::retrieval_query(TaskType::RetrievalQuery, "RETRIEVAL_QUERY")]
#[case::retrieval_document(TaskType::RetrievalDocument, "RETRIEVAL_DOCUMENT")]
#[case::semantic_similarity(TaskType::SemanticSimilarity, "SEMANTIC_SIMILARITY")]
#[case::classification(TaskType::Classification, "CLASSIFICATION")]
#[case::clustering(TaskType::Clustering, "CLUSTERING")]
#[case::question_answering(TaskType::QuestionAnswering, "QUESTION_ANSWERING")]
#[case::fact_verification(TaskType::FactVerification, "FACT_VERIFICATION")]
#[case::code_retrieval_query(TaskType::CodeRetrievalQuery, "CODE_RETRIEVAL_QUERY")]
fn task_types_round_trip(#[case] kind: TaskType, #[case] text: &str) {
    assert_eq!(kind.as_str(), text);
    assert_eq!(TaskType::parse(text), Some(kind));
}

#[rstest]
#[case::unknown("classification")]
#[case::empty("")]
#[case::near("RETRIEVAL")]
fn unknown_task_types_are_rejected(#[case] value: &str) {
    assert_eq!(TaskType::parse(value), None);
}
