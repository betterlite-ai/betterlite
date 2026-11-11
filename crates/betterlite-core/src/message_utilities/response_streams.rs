use thiserror::Error;

pub mod iterators;
pub mod text_completion;

#[derive(Debug, Error, PartialEq)]
pub enum StreamError {
    #[error("the stream yielded no chunk where one is required")]
    MissingChunk,
    #[error("the stream chunk is not a JSON object")]
    NotAnObject,
    #[error("the stream chunk has no first choice")]
    NoChoices,
    #[error("the stream choice has no delta")]
    NoDelta,
    #[error("the response choice has no message")]
    NoMessage,
    #[error("a delay of {seconds} seconds cannot be a duration")]
    Delay { seconds: f64 },
}
