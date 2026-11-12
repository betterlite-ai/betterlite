use betterlite_error::error::details::ResponseDetails;

#[derive(Debug, thiserror::Error)]
pub enum CompletionOptionsError {
    #[error("{}", .0.message)]
    InvalidControlOption(Box<ResponseDetails<400>>),
}
