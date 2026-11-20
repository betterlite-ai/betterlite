use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::evals::error::EvalsError;
use thiserror::Error;

// Everything a handler can fail with: the provider config's own error, a pre-call that stopped the call, and a body the pre-call could not record
#[derive(Debug, Error)]
pub enum EvalHttpError {
    #[error(transparent)]
    Api(#[from] EvalsError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error("the eval request could not be recorded")]
    Record {
        #[source]
        source: serde_json::Error,
    },
}
