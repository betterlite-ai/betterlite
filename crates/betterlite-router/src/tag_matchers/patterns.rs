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
            match build(source) {
                Ok(regex) => compiled.push(CompiledPattern {
                    source: source.clone(),
                    regex,
                }),
                Err(error) => warnings.push(TagWarning {
                    kind: classify(source, &error),
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

fn classify(source: &str, error: &Error) -> WarningKind {
    if uses_named_escape(source) || !matches!(error, Error::ParseError(..)) {
        return WarningKind::UnsupportedPattern;
    }
    WarningKind::InvalidPattern
}

// Python's \N{NAME} is a valid escape fancy-regex's parser refuses; an escaped backslash before it is not one
fn uses_named_escape(source: &str) -> bool {
    let mut characters = source.chars().peekable();
    let mut escaped = false;
    while let Some(character) = characters.next() {
        if character == '\\' {
            escaped = !escaped;
            continue;
        }
        if character == 'N' && escaped && characters.peek() == Some(&'{') {
            return true;
        }
        escaped = false;
    }
    false
}
