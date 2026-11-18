use std::io::Error as IoError;
use std::num::ParseIntError;
use std::string::FromUtf8Error;

#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    #[error("the fixture socket failed")]
    Io(#[from] IoError),
    #[error("the request is not UTF-8")]
    NotText(#[from] FromUtf8Error),
    #[error("the content-length is not a number")]
    ContentLength(#[from] ParseIntError),
    #[error("the connection closed before a request arrived")]
    NoRequest,
    #[error("the fixture thread panicked")]
    Panicked,
}
