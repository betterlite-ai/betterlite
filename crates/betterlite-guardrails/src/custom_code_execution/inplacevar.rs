use super::value::Value;
use thiserror::Error;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InplaceOp {
    Add,
    Subtract,
    Multiply,
    TrueDivide,
    FloorDivide,
    Modulo,
    Power,
    MatrixMultiply,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
}
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InplaceError {
    #[error("augmented assignment {symbol:?} is not supported")]
    UnsupportedOperator { symbol: String },
    #[error("unsupported operand types for {symbol}: {left} and {right}")]
    UnsupportedOperand {
        symbol: &'static str,
        left: &'static str,
        right: &'static str,
    },
    #[error("division by zero")]
    DivideByZero,
    #[error("numeric overflow in {symbol}")]
    Overflow { symbol: &'static str },
    #[error("text or list repetition is longer than the sandbox allows")]
    RepetitionTooLong,
}
impl InplaceOp {
    pub const ALL: [Self; 13] = [
        Self::Add,
        Self::Subtract,
        Self::Multiply,
        Self::TrueDivide,
        Self::FloorDivide,
        Self::Modulo,
        Self::Power,
        Self::MatrixMultiply,
        Self::BitAnd,
        Self::BitOr,
        Self::BitXor,
        Self::ShiftLeft,
        Self::ShiftRight,
    ];
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+=",
            Self::Subtract => "-=",
            Self::Multiply => "*=",
            Self::TrueDivide => "/=",
            Self::FloorDivide => "//=",
            Self::Modulo => "%=",
            Self::Power => "**=",
            Self::MatrixMultiply => "@=",
            Self::BitAnd => "&=",
            Self::BitOr => "|=",
            Self::BitXor => "^=",
            Self::ShiftLeft => "<<=",
            Self::ShiftRight => ">>=",
        }
    }
    pub fn from_symbol(symbol: &str) -> Result<Self, InplaceError> {
        Self::ALL
            .into_iter()
            .find(|operator| operator.symbol() == symbol)
            .ok_or_else(|| InplaceError::UnsupportedOperator {
                symbol: symbol.to_owned(),
            })
    }
}
mod operators;
use operators::numeric;
use operators::sequence::sequence;
pub fn inplacevar(op: InplaceOp, left: &Value, right: &Value) -> Result<Value, InplaceError> {
    match op {
        InplaceOp::Add | InplaceOp::Multiply | InplaceOp::BitOr | InplaceOp::Modulo => {
            sequence(op, left, right).unwrap_or_else(|| numeric(op, left, right))
        }
        other @ (InplaceOp::Subtract
        | InplaceOp::TrueDivide
        | InplaceOp::FloorDivide
        | InplaceOp::Power
        | InplaceOp::MatrixMultiply
        | InplaceOp::BitAnd
        | InplaceOp::BitXor
        | InplaceOp::ShiftLeft
        | InplaceOp::ShiftRight) => numeric(other, left, right),
    }
}
