use betterlite_models::judge_primitives::error::JudgePrimitivesError;

#[derive(Debug, thiserror::Error)]
pub enum JudgeCallError<E> {
    #[error("the judge model call failed: {source}")]
    Completion {
        #[source]
        source: E,
    },
    #[error("the judge model returned no choices")]
    NoChoices,
    #[error("the judge model reply is not a verdict: {source}")]
    Verdict {
        #[source]
        source: JudgePrimitivesError,
    },
}
