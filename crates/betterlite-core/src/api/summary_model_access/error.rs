#[derive(Debug, thiserror::Error)]
pub enum SummaryAccessError {
    #[error("the principal's model-access fields cannot be read")]
    Principal(#[source] serde_json::Error),
}

// _can_object_call_model raises a ProxyException for a denial and a bare Exception for everything else
#[derive(Debug, thiserror::Error)]
pub enum PolicyFailure<Failure> {
    // compact.py returns false without logging the ProxyException
    #[error("the model grant denies the requested model")]
    Denied,
    // the warning prints the exception's own text
    #[error("{0}")]
    Unexpected(Failure),
}
