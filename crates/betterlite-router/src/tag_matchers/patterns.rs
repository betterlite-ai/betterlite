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
    pub regex: Option<Regex>,
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
                    regex: Some(regex),
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
        for pattern in &self.compiled {
            match pattern.find(headers) {
                PatternOutcome::Matched => {
                    return Search {
                        matched: Some(pattern.source.clone()),
                        warnings,
                    };
                }
                PatternOutcome::Missing => {}
                PatternOutcome::Limit => warnings.push(TagWarning {
                    kind: WarningKind::MatchLimitExceeded,
                    pattern: pattern.source.clone(),
                }),
            }
        }
        Search {
            matched: None,
            warnings,
        }
    }
}

impl CompiledPattern {
    fn find(&self, headers: &[String]) -> PatternOutcome {
        let Some(regex) = &self.regex else {
            return PatternOutcome::Missing;
        };
        for header in headers {
            match regex.find(header) {
                Ok(Some(_)) => return PatternOutcome::Matched,
                Ok(None) => {}
                Err(_) => return PatternOutcome::Limit,
            }
        }
        PatternOutcome::Missing
    }
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
