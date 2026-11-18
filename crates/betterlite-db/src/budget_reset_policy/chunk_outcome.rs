#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChunkOutcome {
    pub fetched: usize,
    pub advanced: usize,
}

// _NO_PROGRESS: the sentinel a failed chunk returns so the phase stops instead of re-reading the same rows.
pub const NO_PROGRESS: ChunkOutcome = ChunkOutcome {
    fetched: 0,
    advanced: 0,
};

mod tests;
