use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RepairError {
    #[error("a message has no readable get method")]
    NoGetMethod,
    #[error("an object's get method failed")]
    GetterFailed,
    #[error("{field} has a value of the wrong type")]
    WrongType { field: &'static str },
}
