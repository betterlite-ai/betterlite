use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProtocolCostError {
    #[error("{field} is not a finite number that fits a monetary value")]
    InvalidMoney { field: &'static str },
    #[error("the protocol charge is too large to represent")]
    Overflow,
}
