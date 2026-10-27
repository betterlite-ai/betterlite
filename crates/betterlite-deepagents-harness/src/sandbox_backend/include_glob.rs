mod tests;

use glob::{MatchOptions, Pattern, PatternError};
use thiserror::Error;

// Deep Agents rejects patterns that would produce more than 1,000 alternatives.
const BRACE_LIMIT: usize = 1000;

const MATCH_OPTIONS: MatchOptions = MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: true,
};

#[derive(Debug, Error)]
pub enum InvalidGlobPattern {
    #[error("Path traversal not allowed in glob pattern '{pattern}'")]
    Traversal { pattern: String },
    #[error("Invalid glob pattern '{pattern}': {source}")]
    Pattern {
        pattern: String,
        #[source]
        source: PatternError,
    },
    #[error("Invalid glob pattern '{pattern}': more than {BRACE_LIMIT} brace expansions")]
    TooManyExpansions { pattern: String },
}

#[derive(Debug)]
pub struct IncludeGlob {
    alternatives: Vec<Pattern>,
    anchored: bool,
}

impl IncludeGlob {
    pub fn compile(pattern: &str) -> Result<Self, InvalidGlobPattern> {
        if pattern
            .replace('\\', "/")
            .split('/')
            .any(|segment| segment == "..")
        {
            return Err(InvalidGlobPattern::Traversal {
                pattern: pattern.to_owned(),
            });
        }
        let expanded = brace_expansion::expand_with_max(
            pattern.trim_start_matches('/'),
            BRACE_LIMIT.saturating_add(1),
        );
        if expanded.len() > BRACE_LIMIT {
            return Err(InvalidGlobPattern::TooManyExpansions {
                pattern: pattern.to_owned(),
            });
        }
        let alternatives = expanded
            .iter()
            .map(|alternative| Pattern::new(alternative))
            .collect::<Result<Vec<Pattern>, PatternError>>()
            .map_err(|source| InvalidGlobPattern::Pattern {
                pattern: pattern.to_owned(),
                source,
            })?;
        Ok(Self {
            alternatives,
            anchored: pattern.contains('/'),
        })
    }

    #[must_use]
    pub fn matches(&self, root: &str, real: &str) -> bool {
        let relative_path = real
            .strip_prefix(root)
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(real);
        let subject = if self.anchored {
            relative_path
        } else {
            relative_path.rsplit('/').next().unwrap_or(relative_path)
        };
        self.alternatives
            .iter()
            .any(|alternative| alternative.matches_with(subject, MATCH_OPTIONS))
    }
}
