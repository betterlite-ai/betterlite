use thiserror::Error;

use super::environment_error::EnvironmentError;
use super::request::RequestError;
use super::url_error::UrlError;

#[derive(Debug, Error)]
pub enum ChatError {
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    #[error(transparent)]
    Url(#[from] UrlError),
    #[error(transparent)]
    Request(#[from] RequestError),
}
