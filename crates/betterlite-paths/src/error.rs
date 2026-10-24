use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathError {
    #[error("the path has {bytes} bytes, more than the limit of {limit}")]
    TooLong { bytes: usize, limit: usize },
    #[error("the path has more than {limit} segments")]
    TooDeep { limit: usize },
    #[error("the path has more than {limit} array selectors")]
    TooManySelectors { limit: usize },
    #[error("deleting the path would visit more than {limit} values")]
    TooMuchWork { limit: usize },
}
