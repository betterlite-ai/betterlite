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
        let translated = translate_flag_syntax(source)?;
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

// Python's (?a) is Rust's (?-u): ASCII classes, ASCII boundaries and ASCII-only case folding
fn translate_flag_syntax(source: &str) -> Result<String, PatternError> {
    if source.contains("(?-a") || source.contains("(?-u") {
        return Err(PatternError::UnsupportedFlags);
    }
    let flag_group = regex::Regex::new(r"\(\?([aiLmsux]+)(?:-([imsx]*))?([:)])")?;
    Ok(flag_group
        .replace_all(source, translate_flag_group)
        .into_owned())
}

fn translate_flag_group(captures: &regex::Captures<'_>) -> String {
    let enabled = &captures[1];
    let disabled = captures.get(2).map_or("", |matched| matched.as_str());
    let terminator = &captures[3];
    let mut translated = String::new();
    let mut ascii = false;
    for character in enabled.chars() {
        match character {
            'a' => ascii = true,
            'u' => translated.push('u'),
            other => translated.push(other),
        }
    }
    let mut disables = String::from(disabled);
    if ascii {
        disables.push('u');
    }
    if disables.is_empty() {
        return format!("(?{translated}{terminator}");
    }
    format!("(?{translated}-{disables}{terminator}")
}
