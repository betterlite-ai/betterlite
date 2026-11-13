#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskType {
    RetrievalQuery,
    RetrievalDocument,
    SemanticSimilarity,
    Classification,
    Clustering,
    QuestionAnswering,
    FactVerification,
    CodeRetrievalQuery,
}

impl TaskType {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "RETRIEVAL_QUERY" => Some(Self::RetrievalQuery),
            "RETRIEVAL_DOCUMENT" => Some(Self::RetrievalDocument),
            "SEMANTIC_SIMILARITY" => Some(Self::SemanticSimilarity),
            "CLASSIFICATION" => Some(Self::Classification),
            "CLUSTERING" => Some(Self::Clustering),
            "QUESTION_ANSWERING" => Some(Self::QuestionAnswering),
            "FACT_VERIFICATION" => Some(Self::FactVerification),
            "CODE_RETRIEVAL_QUERY" => Some(Self::CodeRetrievalQuery),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RetrievalQuery => "RETRIEVAL_QUERY",
            Self::RetrievalDocument => "RETRIEVAL_DOCUMENT",
            Self::SemanticSimilarity => "SEMANTIC_SIMILARITY",
            Self::Classification => "CLASSIFICATION",
            Self::Clustering => "CLUSTERING",
            Self::QuestionAnswering => "QUESTION_ANSWERING",
            Self::FactVerification => "FACT_VERIFICATION",
            Self::CodeRetrievalQuery => "CODE_RETRIEVAL_QUERY",
        }
    }
}
