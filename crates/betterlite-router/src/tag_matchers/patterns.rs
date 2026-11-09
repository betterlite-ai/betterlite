use fancy_regex::{Error, Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use self::syntax::unsupported_syntax;

mod syntax;

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
