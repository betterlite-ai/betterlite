use std::path::Path;

// The workspace crates directory plays the role of LiteLLM's package directory
const CRATES_DIR: &str = "crates";

// LiteLLM keeps the last 12 frames, and a creation site is one frame, so it always fits
pub const MAX_FRAMES: usize = 12;

#[must_use]
pub fn litellm_frames(file: &str, line: u32) -> Vec<String> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let crates_dir = manifest_dir.parent().unwrap_or(manifest_dir);
    let path = Path::new(file);
    let Some(relative) = path
        .strip_prefix(crates_dir)
        .or_else(|_| path.strip_prefix(CRATES_DIR))
        .ok()
        .and_then(Path::to_str)
    else {
        return Vec::new();
    };
    vec![format!("{CRATES_DIR}/{relative}:{line}")]
}
