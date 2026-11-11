pub mod to_ir;
pub mod to_openai;

use betterlite_models::decisions::text::TextError;

#[derive(Debug, thiserror::Error)]
pub enum ResponseConversionError {
    #[error("the decision response has {answers} answers for {questions} questions")]
    AnswerCount { answers: usize, questions: usize },
    #[error("a decision score probability value is not a level index")]
    LevelIndex,
    #[error("a decision probability is not a finite number")]
    Probability,
    #[error("a decision token count is not an integer")]
    TokenCount,
    #[error(transparent)]
    Text(#[from] TextError),
}
