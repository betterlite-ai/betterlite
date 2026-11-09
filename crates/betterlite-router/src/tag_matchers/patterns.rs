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
    let mut characters = source.chars().peekable();
    let mut escaped = false;
    let mut in_class = false;
    let mut class_start = false;
    while let Some(character) = characters.next() {
        if escaped {
            escaped = false;
            if unsupported_escape(character, characters.peek(), in_class) {
                return true;
            }
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if in_class {
            match class_step(character, characters.peek(), class_start) {
                ClassStep::Unsupported => return true,
                ClassStep::Start => {}
                ClassStep::Close => in_class = false,
                ClassStep::Literal => class_start = false,
            }
            continue;
        }
        if character == '[' {
            in_class = true;
            class_start = true;
            continue;
        }
        if character == '(' && characters.peek() == Some(&'?') {
            characters.next();
            if characters.peek() == Some(&'(') {
                return true;
            }
        }
    }
    false
}

fn unsupported_escape(character: char, next: Option<&char>, in_class: bool) -> bool {
    if character == 'N' && next == Some(&'{') {
        return true;
    }
    if in_class {
        return false;
    }
    if character == '<' || character == '>' {
        return true;
    }
    character == '0' && next.is_some_and(|digit| ('0'..='7').contains(digit))
}

enum ClassStep {
    Unsupported,
    Start,
    Close,
    Literal,
}

fn class_step(character: char, next: Option<&char>, class_start: bool) -> ClassStep {
    if character == '[' {
        if next == Some(&':') || !class_start {
            return ClassStep::Unsupported;
        }
        return ClassStep::Literal;
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
