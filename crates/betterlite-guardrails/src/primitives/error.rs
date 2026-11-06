use thiserror::Error;

#[derive(Debug, Error)]
pub enum PrimitiveError {
    #[error("primitive {primitive} takes {expected} arguments but got {given}")]
    TooManyArguments {
        primitive: &'static str,
        expected: usize,
        given: usize,
    },
    #[error("primitive {primitive} got the argument {name} twice")]
    DuplicateArgument {
        primitive: &'static str,
        name: String,
    },
    #[error("primitive {primitive} got arguments of the wrong shape")]
    Arguments {
        primitive: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("primitive arguments could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the {language} code pattern does not compile")]
    CodePattern {
        language: &'static str,
        #[source]
        source: regex::Error,
    },
    #[error("the url pattern does not compile")]
    UrlPattern {
        #[source]
        source: regex::Error,
    },
}
