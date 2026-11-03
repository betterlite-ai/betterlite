use pcre2::bytes;

#[derive(Debug)]
pub enum Pattern {
    Fancy(fancy_regex::Regex),
    Pcre(bytes::Regex),
}

#[derive(Debug, thiserror::Error)]
pub enum PatternError {
    #[error(transparent)]
    Fancy(#[from] fancy_regex::Error),
    #[error(transparent)]
    Pcre(#[from] pcre2::Error),
}

impl Pattern {
    pub fn is_match(&self, text: &str) -> Result<bool, PatternError> {
        match self {
            Self::Fancy(regex) => regex.is_match(text).map_err(PatternError::Fancy),
            Self::Pcre(regex) => regex.is_match(text.as_bytes()).map_err(PatternError::Pcre),
        }
    }
}
