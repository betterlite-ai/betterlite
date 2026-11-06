use thiserror::Error;
use tree_sitter::LanguageError;

// compile_sandboxed raises this for a syntax error, a forbidden construct, or a grammar that cannot load.
#[derive(Debug, Error)]
pub enum CompileError {
    #[error("{filename}: line {line}: {message}")]
    Syntax {
        filename: String,
        line: u32,
        message: String,
    },
    #[error("{filename}: line {line}: {message}")]
    Forbidden {
        filename: String,
        line: u32,
        message: String,
    },
    #[error("could not load the Python grammar")]
    Grammar {
        #[source]
        source: LanguageError,
    },
}

impl CompileError {
    #[must_use]
    pub fn syntax(filename: &str, line: u32, message: &str) -> Self {
        Self::Syntax {
            filename: filename.to_owned(),
            line,
            message: message.to_owned(),
        }
    }

    #[must_use]
    pub fn forbidden(filename: &str, line: u32, message: String) -> Self {
        Self::Forbidden {
            filename: filename.to_owned(),
            line,
            message,
        }
    }
}
