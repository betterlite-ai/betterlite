use regex::bytes::{Regex, RegexBuilder};

#[derive(Debug)]
pub enum Pattern {
    Backtracking(fancy_regex::Regex),
    Linear(Regex),
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

#[derive(Clone, Copy, Default)]
struct ScopeFlags {
    multiline: bool,
    ascii: bool,
}

impl ScopeFlags {
    // enters the group that starts the text and gives the flags to restore when it closes; a flags-only (?m) lasts to the end of its enclosing group
    fn open(&mut self, flag_group: &regex::Regex, group: &str) -> Self {
        let outer = *self;
        let Some(captures) = flag_group.captures(group) else {
            return outer;
        };
        let enabled = captures.get(1).map_or("", |matched| matched.as_str());
        let disabled = captures.get(2).map_or("", |matched| matched.as_str());
        *self = Self {
            multiline: (outer.multiline || enabled.contains('m')) && !disabled.contains('m'),
            ascii: (outer.ascii || enabled.contains('a')) && !enabled.contains('u'),
        };
        let flags_only = captures
            .get(3)
            .is_some_and(|terminator| terminator.as_str() == ")");
        if flags_only { *self } else { outer }
    }
}

impl Pattern {
    pub fn compile(source: &str) -> Result<Self, PatternError> {
        let flag_group = regex::Regex::new(r"^\(\?([aiLmsux]*)(?:-([imsx]*))?([:)])")?;
        let lookahead = translate_scoped_syntax(source, r"(?=\n?\z)", &flag_group);
        if let Ok(regex) = fancy_regex::RegexBuilder::new(&lookahead)
            .case_insensitive(true)
            .build()
        {
            return Ok(Self::Backtracking(regex));
        }
        let linear =
            translate_flag_syntax(&translate_scoped_syntax(source, r"(?:\n?\z)", &flag_group))?;
        let regex = RegexBuilder::new(&linear).case_insensitive(true).build()?;
        Ok(Self::Linear(regex))
    }

    pub fn is_match(&self, text: &str) -> Result<bool, PatternError> {
        match self {
            Self::Backtracking(regex) => regex.is_match(text).map_err(PatternError::Backtracking),
            Self::Linear(regex) => Ok(regex.is_match(text.as_bytes())),
        }
    }
}

// Python's $ also matches before one final newline unless (?m) is on, and Python's \Z is Rust's \z
fn translate_scoped_syntax(source: &str, end: &str, flag_group: &regex::Regex) -> String {
    let mut translated = String::with_capacity(source.len());
    let mut flags = ScopeFlags::default();
    let mut enclosing = Vec::new();
    let mut in_class = false;
    let mut characters = source.char_indices();
    while let Some((index, character)) = characters.next() {
        match character {
            '\\' => push_escape(
                &mut translated,
                characters.next().map(|(_, escaped)| escaped),
                flags.ascii && !in_class,
            ),
            '[' => {
                in_class = true;
                translated.push(character);
            }
            ']' => {
                in_class = false;
                translated.push(character);
            }
            _ if in_class => translated.push(character),
            '(' => {
                enclosing.push(flags.open(flag_group, source.get(index..).unwrap_or_default()));
                translated.push(character);
            }
            ')' => {
                flags = enclosing.pop().unwrap_or(flags);
                translated.push(character);
            }
            '$' if !flags.multiline => translated.push_str(end),
            '.' => translated.push_str(if flags.ascii { "(?u:.)" } else { "." }),
            _ => translated.push(character),
        }
    }
    translated
}

// Under (?a) Python's ., \D, \S and \W match one character, where the linear engine's (?-u) forms match one byte
fn push_escape(translated: &mut String, escaped: Option<char>, ascii: bool) {
    match escaped {
        Some('D') if ascii => translated.push_str("(?u-i:[^0-9])"),
        Some('S') if ascii => translated.push_str(r"(?u-i:[^\t\n\x0B\x0C\r\x20])"),
        Some('W') if ascii => translated.push_str("(?u-i:[^0-9A-Za-z_])"),
        Some('Z') => translated.push_str(r"\z"),
        Some(other) => {
            translated.push('\\');
            translated.push(other);
        }
        None => translated.push('\\'),
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
    let mut turned_off = String::from(disabled);
    if ascii {
        turned_off.push('u');
    }
    if turned_off.is_empty() {
        return format!("(?{translated}{terminator}");
    }
    format!("(?{translated}-{turned_off}{terminator}")
}
