#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct JudgeDispatchError {
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum JudgeCallError {
    #[error("judge model {model} resolves to no router deployment and no provider")]
    Nothing { model: String },
    #[error("the judge call to {model} failed: {source}")]
    Dispatch {
        model: String,
        #[source]
        source: JudgeDispatchError,
    },
}
