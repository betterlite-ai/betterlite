use thiserror::Error;

// compile_sandboxed raises this for a Python syntax error and for a construct the sandbox forbids.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{filename}: line {line}: {message}")]
pub struct CompileError {
    pub filename: String,
    pub line: u32,
    pub message: String,
}

impl CompileError {
    #[must_use]
    pub fn new(filename: &str, line: u32, message: String) -> Self {
        Self {
            filename: filename.to_owned(),
            line,
            message,
        }
    }
}
