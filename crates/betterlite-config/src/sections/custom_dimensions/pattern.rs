mod cost;
mod model;
mod normalize;

pub const MAX_CUSTOM_PATTERN_REPEAT: u64 = 64;
pub const MAX_CUSTOM_PATTERN_WORK: u64 = 2048;
pub const MAX_CUSTOM_PATTERN_DEPTH: u64 = 16;
pub const MAX_REPEAT: u64 = 4_294_967_295;
pub const MAX_CUSTOM_PATTERN_INPUT: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternProblem {
    Syntax,
    Unsupported,
    Repeat,
    Depth,
    Work,
}

impl PatternProblem {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Syntax => "is not a valid regex",
            Self::Unsupported => "contains an unsupported regex construct",
            Self::Repeat => {
                "requires a single character or class repeated at most 64 times; use {n,m} instead of *, + or {n,}"
            }
            Self::Depth => "nests deeper than 16 levels",
            Self::Work => "exceeds the per-pattern regex work budget",
        }
    }
}

// the pattern as the matcher engine reads it, and its measured work
pub fn pattern_work(pattern: &str) -> Result<(String, u64), PatternProblem> {
    if pattern.chars().count() > MAX_CUSTOM_PATTERN_INPUT {
        return Err(PatternProblem::Syntax);
    }
    let normalized = normalize::pattern(pattern)?;
    let nodes = model::parse(&normalized)?;
    Ok((normalized, cost::sequence_cost(&nodes, 0)?.1))
}
