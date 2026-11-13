use thiserror::Error;

#[derive(Debug, Error)]
pub enum PatternError {
    #[error("the regex failed")]
    Regex(#[from] fancy_regex::Error),
    #[error("the replacement template is not valid")]
    Template,
}
