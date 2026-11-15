#[must_use]
pub fn selects(model: &str) -> bool {
    // the exact names only: no prefix stripping, case folding or version extension
    matches!(model, "claude-2" | "claude-instant-1")
}
