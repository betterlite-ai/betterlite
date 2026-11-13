#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MethodAccess<'method> {
    NoRequest,
    Unavailable,
    Other(String),
    NonString,
    Text(&'method str),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MethodAccessError {
    #[error("the method accessor failed: {0}")]
    Other(String),
}

pub fn get_request_method(access: MethodAccess<'_>) -> Result<Option<String>, MethodAccessError> {
    match access {
        MethodAccess::NoRequest | MethodAccess::Unavailable | MethodAccess::NonString => Ok(None),
        MethodAccess::Other(message) => Err(MethodAccessError::Other(message)),
        MethodAccess::Text(method) => Ok(Some(method.to_uppercase())),
    }
}
