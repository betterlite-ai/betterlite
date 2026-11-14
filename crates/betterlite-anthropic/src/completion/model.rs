#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyModel {
    Claude2,
    ClaudeInstant1,
}

// the exact names only: no prefix stripping, case folding or version extension
#[must_use]
pub fn select(model: &str) -> Option<LegacyModel> {
    match model {
        "claude-2" => Some(LegacyModel::Claude2),
        "claude-instant-1" => Some(LegacyModel::ClaudeInstant1),
        _ => None,
    }
}
