#[derive(Debug)]
pub enum Pattern {
    Backtracking(fancy_regex::Regex),
    Linear(regex::bytes::Regex),
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

#[derive(Clone, Copy)]
enum AnchorForm {
    Lookahead,
    Consuming,
}

impl Pattern {
    pub fn compile(source: &str) -> Result<Self, PatternError> {
        let lookahead = translate_anchors(source, AnchorForm::Lookahead);
        if let Ok(regex) = fancy_regex::RegexBuilder::new(&lookahead)
            .case_insensitive(true)
            .build()
        {
            return Ok(Self::Backtracking(regex));
        }
        let linear = translate_flag_syntax(&translate_anchors(source, AnchorForm::Consuming))?;
        let regex = regex::bytes::RegexBuilder::new(&linear)
            .case_insensitive(true)
            .build()?;
        Ok(Self::Linear(regex))
    }

    pub fn is_match(&self, text: &str) -> Result<bool, PatternError> {
        match self {
            Self::Backtracking(regex) => regex.is_match(text).map_err(PatternError::Backtracking),
            Self::Linear(regex) => Ok(regex.is_match(text.as_bytes())),
        }
    }
}

// Python's $ also matches before one final newline and \Z only ignores that newline; Rust's $ and \z are the absolute end
fn translate_anchors(source: &str, form: AnchorForm) -> String {
    let end = match form {
        AnchorForm::Lookahead => "(?=\\n?\\z)",
        AnchorForm::Consuming => "(?:\\n?\\z)",
    };
    let mut translated = String::with_capacity(source.len());
    let mut characters = source.chars().peekable();
    let mut in_class = false;
    while let Some(character) = characters.next() {
        match character {
            '\\' => {
                translated.push(character);
                if let Some(escaped) = characters.next() {
                    translated.push(if escaped == 'Z' { 'z' } else { escaped });
                }
            }
            '[' => {
                in_class = true;
                translated.push(character);
            }
            ']' => {
                in_class = false;
                translated.push(character);
            }
            '$' if !in_class => translated.push_str(end),
            _ => translated.push(character),
        }
    }
    translated
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
