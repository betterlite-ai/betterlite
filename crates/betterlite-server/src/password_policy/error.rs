use thiserror::Error;

pub const STATUS: u16 = 400;
pub const ERROR_TYPE: &str = "validation_error";
pub const PARAM: &str = "password";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PasswordError {
    #[error("Password does not meet the required policy: must {requirements}.")]
    Policy { requirements: String },
    #[error(
        "This password appears in known data breaches and cannot be used. Please choose a different password."
    )]
    Breached,
}
