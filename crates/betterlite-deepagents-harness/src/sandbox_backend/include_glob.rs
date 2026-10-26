mod tests;

use glob::{MatchOptions, Pattern, PatternError};
use thiserror::Error;

// wcmatch's brace expansion limit, the matcher deepagents compiles with
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

struct BraceGroup<'pattern> {
    prefix: &'pattern str,
    alternatives: Vec<&'pattern str>,
    suffix: &'pattern str,
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
        let mut expanded = Vec::new();
        expand_braces(pattern, pattern.trim_start_matches('/'), &mut expanded)?;
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

fn expand_braces(
    original: &str,
    pattern: &str,
    expanded: &mut Vec<String>,
) -> Result<(), InvalidGlobPattern> {
    let Some(group) = first_group(pattern) else {
        if expanded.len() >= BRACE_LIMIT {
            return Err(InvalidGlobPattern::TooManyExpansions {
                pattern: original.to_owned(),
            });
        }
        expanded.push(pattern.to_owned());
        return Ok(());
    };
    for alternative in group.alternatives {
        let joined = format!("{}{alternative}{}", group.prefix, group.suffix);
        expand_braces(original, &joined, expanded)?;
    }
    Ok(())
}

fn first_group(pattern: &str) -> Option<BraceGroup<'_>> {
    let mut escaped = false;
    for (index, byte) in pattern.bytes().enumerate() {
        match (escaped, byte) {
            (true, _) => escaped = false,
            (false, b'\\') => escaped = true,
            (false, b'{') => {
                if let Some(group) = group_at(pattern, index) {
                    return Some(group);
                }
            }
            (false, _) => {}
        }
    }
    None
}

fn group_at(pattern: &str, open: usize) -> Option<BraceGroup<'_>> {
    let mut depth = 0_usize;
    let mut escaped = false;
    let mut cuts = vec![open];
    for (index, byte) in pattern.bytes().enumerate().skip(open.saturating_add(1)) {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'{' => depth = depth.saturating_add(1),
            b'}' if depth == 0 => return split_group(pattern, &cuts, index),
            b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => cuts.push(index),
            _ => {}
        }
    }
    None
}

fn split_group<'pattern>(
    pattern: &'pattern str,
    cuts: &[usize],
    close: usize,
) -> Option<BraceGroup<'pattern>> {
    let open = *cuts.first()?;
    if cuts.len() < 2 {
        return None;
    }
    let ends = cuts.iter().skip(1).copied().chain([close]);
    let alternatives = cuts
        .iter()
        .zip(ends)
        .map(|(cut, end)| pattern.get(cut.saturating_add(1)..end))
        .collect::<Option<Vec<&str>>>()?;
    Some(BraceGroup {
        prefix: pattern.get(..open)?,
        alternatives,
        suffix: pattern.get(close.saturating_add(1)..)?,
    })
}
