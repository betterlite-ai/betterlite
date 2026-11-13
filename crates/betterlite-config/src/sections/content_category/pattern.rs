#[derive(Debug)]
pub enum Pattern {
    Backtracking(fancy_regex::Regex),
    Linear(regex::Regex),
}

#[derive(Debug, thiserror::Error)]
pub enum PatternError {
    #[error(transparent)]
    Backtracking(#[from] fancy_regex::Error),
    #[error(transparent)]
    Linear(#[from] regex::Error),
    #[error("the phrase pattern turns off Python's 'a' or 'u' flag")]
    UnsupportedFlags,
}

impl Pattern {
    pub fn compile(source: &str) -> Result<Self, PatternError> {
        if let Ok(regex) = fancy_regex::RegexBuilder::new(source)
            .case_insensitive(true)
            .build()
        {
            return Ok(Self::Backtracking(regex));
        }
        let translated = translate_python_flags(source)?;
        let regex = regex::RegexBuilder::new(&translated)
            .case_insensitive(true)
            .build()?;
        Ok(Self::Linear(regex))
    }

    pub fn is_match(&self, text: &str) -> Result<bool, PatternError> {
        match self {
            Self::Backtracking(regex) => regex.is_match(text).map_err(PatternError::Backtracking),
            Self::Linear(regex) => Ok(regex.is_match(text)),
        }
    }
}

// Python's (?a) flag is Rust's (?-u): ASCII classes, boundaries and case folding, scoped the same way
fn translate_python_flags(source: &str) -> Result<String, PatternError> {
    if source.contains("(?-a") || source.contains("(?-u") {
        return Err(PatternError::UnsupportedFlags);
    }
    Ok(source.replace("(?a", "(?-u"))
}
