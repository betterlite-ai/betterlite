mod cost;
mod model;
mod normalize;

const MAX_CUSTOM_PATTERN_REPEAT: u64 = 64;
const MAX_CUSTOM_PATTERN_WORK: u64 = 2048;
const MAX_CUSTOM_PATTERN_DEPTH: u64 = 16;
const MAX_REPEAT: u64 = 0xFFFF_FFFF;
const MAX_CUSTOM_PATTERN_INPUT: usize = 4096;

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

pub fn pattern_work(pattern: &str) -> Result<(String, u64), PatternProblem> {
    if pattern.chars().count() > MAX_CUSTOM_PATTERN_INPUT {
        return Err(PatternProblem::Syntax);
    }
    let normalized = normalize::pattern(pattern, false)?;
    let nodes = model::parse(&normalized)?;
    let work = cost::sequence_cost(&nodes, 0)?.1;
    Ok((normalize::pattern(pattern, true)?, work))
}
