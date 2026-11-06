use thiserror::Error;

use super::value::Value;

// Every augmented-assignment operator Python's ast can carry, spelled as `_inplacevar_` receives it.
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

    // A symbol outside ALL is a SyntaxError, matching the source's `_inplacevar_` raising for an unknown operator.
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

use operators::{
    add, bits, floor_divide, modulo, multiply, power, subtract, true_divide, unsupported,
};

// `x += y` compiles to `x = _inplacevar_("+=", x, y)`, so this is the function that call resolves to.
pub fn inplacevar(op: InplaceOp, left: &Value, right: &Value) -> Result<Value, InplaceError> {
    match op {
        InplaceOp::Add => add(left, right),
        InplaceOp::Subtract => subtract(left, right),
        InplaceOp::Multiply => multiply(left, right),
        InplaceOp::TrueDivide => true_divide(left, right),
        InplaceOp::FloorDivide => floor_divide(left, right),
        InplaceOp::Modulo => modulo(left, right),
        InplaceOp::Power => power(left, right),
        InplaceOp::MatrixMultiply => Err(unsupported(op, left, right)),
        InplaceOp::BitAnd
        | InplaceOp::BitOr
        | InplaceOp::BitXor
        | InplaceOp::ShiftLeft
        | InplaceOp::ShiftRight => bits(op, left, right),
    }
}
