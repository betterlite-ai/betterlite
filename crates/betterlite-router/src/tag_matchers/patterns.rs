use std::iter::Peekable;
use std::str::Chars;

use fancy_regex::{Error, Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

pub const MAX_PATTERN_BYTES: usize = 2048;
pub const MAX_MATCH_STEPS: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    InvalidPattern,
    UnsupportedPattern,
    PatternTooLarge,
    MatchLimitExceeded,
}

impl WarningKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::InvalidPattern => "invalid_pattern",
            Self::UnsupportedPattern => "unsupported_pattern",
            Self::PatternTooLarge => "pattern_too_large",
            Self::MatchLimitExceeded => "match_limit_exceeded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagWarning {
    pub kind: WarningKind,
    pub pattern: String,
}

#[derive(Debug)]
pub struct Patterns {
    pub compiled: Vec<CompiledPattern>,
    pub warnings: Vec<TagWarning>,
}

#[derive(Debug)]
pub struct CompiledPattern {
    pub source: String,
    pub regex: Regex,
}

#[derive(Debug)]
pub struct Search {
    pub matched: Option<String>,
    pub warnings: Vec<TagWarning>,
}

#[derive(Debug, Clone, Copy)]
enum PatternOutcome {
    Matched,
    Missing,
    Limit,
}

impl Patterns {
    #[must_use]
    pub fn compile(sources: &[String]) -> Self {
        let mut compiled = Vec::new();
        let mut warnings = Vec::new();
        for source in sources {
            if source.len() > MAX_PATTERN_BYTES {
                warnings.push(TagWarning {
                    kind: WarningKind::PatternTooLarge,
                    pattern: source.clone(),
                });
                continue;
            }
            if unsupported_syntax(source) {
                warnings.push(TagWarning {
                    kind: WarningKind::UnsupportedPattern,
                    pattern: source.clone(),
                });
                continue;
            }
            match build(source) {
                Ok(regex) => compiled.push(CompiledPattern {
                    source: source.clone(),
                    regex,
                }),
                Err(error) => warnings.push(TagWarning {
                    kind: classify(&error),
                    pattern: source.clone(),
                }),
            }
        }
        Self { compiled, warnings }
    }

    #[must_use]
    pub fn find(&self, headers: &[String]) -> Search {
        let mut warnings = Vec::new();
        let mut matched = None;
        for pattern in &self.compiled {
            match header_outcome(&pattern.regex, headers) {
                PatternOutcome::Matched => {
                    matched = Some(pattern.source.clone());
                    break;
                }
                PatternOutcome::Missing => {}
                PatternOutcome::Limit => warnings.push(TagWarning {
                    kind: WarningKind::MatchLimitExceeded,
                    pattern: pattern.source.clone(),
                }),
            }
        }
        Search { matched, warnings }
    }
}

fn header_outcome(regex: &Regex, headers: &[String]) -> PatternOutcome {
    for header in headers {
        match regex.find(header) {
            Ok(Some(_)) => return PatternOutcome::Matched,
            Ok(None) => {}
            Err(_) => return PatternOutcome::Limit,
        }
    }
    PatternOutcome::Missing
}

fn build(source: &str) -> Result<Regex, Error> {
    // seek stays off so the backtrack limit bounds every pattern, not only the ones without a usable prefilter
    RegexBuilder::new(source)
        .backtrack_limit(MAX_MATCH_STEPS)
        .seek(false)
        .build()
}

const fn classify(error: &Error) -> WarningKind {
    if matches!(error, Error::ParseError(..)) {
        return WarningKind::InvalidPattern;
    }
    WarningKind::UnsupportedPattern
}

// Python and the engine read these constructs differently, so the pattern is skipped instead of reinterpreted
fn unsupported_syntax(source: &str) -> bool {
    let mut scanner = Scanner {
        characters: source.chars().peekable(),
        escaped: false,
        class: ClassState::Outside,
    };
    while let Some(character) = scanner.characters.next() {
        if scanner.step(character) {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClassState {
    Outside,
    Start,
    Body,
}

struct Scanner<'source> {
    characters: Peekable<Chars<'source>>,
    escaped: bool,
    class: ClassState,
}

impl Scanner<'_> {
    fn step(&mut self, character: char) -> bool {
        if self.escaped {
            self.escaped = false;
            return self.escape(character);
        }
        if character == '\\' {
            self.escaped = true;
            return false;
        }
        if self.class != ClassState::Outside {
            return self.class_char(character);
        }
        if character == '[' {
            self.class = ClassState::Start;
            return false;
        }
        character == '(' && self.characters.peek() == Some(&'?') && self.group()
    }

    fn escape(&mut self, character: char) -> bool {
        let next = self.characters.peek().copied();
        if (character == 'N' && next == Some('{')) || character == 'Z' {
            return true;
        }
        if self.class != ClassState::Outside {
            self.class = ClassState::Body;
            return false;
        }
        if character == '<' || character == '>' {
            return true;
        }
        if character == '0' && next.is_some_and(|digit| ('0'..='7').contains(&digit)) {
            return true;
        }
        is_three_digit_octal(character, &self.characters)
    }

    fn group(&mut self) -> bool {
        self.characters.next();
        self.characters.peek() == Some(&'(') || enables_verbose(&self.characters)
    }

    fn class_char(&mut self, character: char) -> bool {
        let class_start = self.class == ClassState::Start;
        match class_step(character, self.characters.peek(), class_start) {
            ClassStep::Unsupported => true,
            ClassStep::Start => false,
            ClassStep::Close => {
                self.class = ClassState::Outside;
                false
            }
            ClassStep::Literal => {
                self.class = ClassState::Body;
                false
            }
        }
    }
}

fn is_three_digit_octal<I>(character: char, characters: &I) -> bool
where
    I: Iterator<Item = char> + Clone,
{
    if !matches!(character, '1'..='3') {
        return false;
    }
    let mut probe = characters.clone();
    let second = probe.next();
    let third = probe.next();
    second.is_some_and(|digit| ('0'..='7').contains(&digit))
        && third.is_some_and(|digit| ('0'..='7').contains(&digit))
}

fn enables_verbose<I>(characters: &I) -> bool
where
    I: Iterator<Item = char> + Clone,
{
    let mut verbose = false;
    let mut disabled = false;
    for character in characters.clone() {
        match character {
            '-' => disabled = true,
            ':' | ')' => return verbose,
            'x' => {
                if !disabled {
                    verbose = true;
                }
            }
            'a' | 'i' | 'L' | 'm' | 's' | 'u' => {}
            _ => return false,
        }
    }
    false
}

enum ClassStep {
    Unsupported,
    Start,
    Close,
    Literal,
}

fn class_step(character: char, next: Option<&char>, class_start: bool) -> ClassStep {
    if character == '[' {
        return ClassStep::Unsupported;
    }
    if character == ']' {
        if class_start {
            return ClassStep::Literal;
        }
        return ClassStep::Close;
    }
    if class_start && character == '^' {
        return ClassStep::Start;
    }
    let doubled = next.is_some_and(|following| {
        (*following == '&' && character == '&')
            || (*following == '~' && character == '~')
            || (*following == '-' && character == '-')
    });
    if doubled {
        return ClassStep::Unsupported;
    }
    ClassStep::Literal
}
