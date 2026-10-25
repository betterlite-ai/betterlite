use std::path::Path;

use backtrace::{Backtrace, BacktraceFrame, BacktraceSymbol};

pub const MAX_FRAMES: usize = 12;

// The workspace crates directory plays the role of LiteLLM's package directory
const CRATES_DIR: &str = "crates";

#[must_use]
pub fn litellm_frames(backtrace: &Backtrace) -> Vec<String> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let crates_dir = manifest_dir.parent().unwrap_or(manifest_dir);
    let mut frames: Vec<String> = backtrace
        .frames()
        .iter()
        .flat_map(BacktraceFrame::symbols)
        .rev()
        .filter_map(|symbol| format_frame(symbol, crates_dir))
        .collect();
    let excess = frames.len().saturating_sub(MAX_FRAMES);
    frames.drain(..excess);
    frames
}

fn format_frame(symbol: &BacktraceSymbol, crates_dir: &Path) -> Option<String> {
    let file = symbol.filename()?;
    let relative = file
        .strip_prefix(crates_dir)
        .or_else(|_| file.strip_prefix(CRATES_DIR))
        .ok()?;
    let line = symbol.lineno()?;
    let name = symbol.name()?;
    Some(format!(
        "{CRATES_DIR}/{}:{line} in {name:#}",
        relative.to_str()?
    ))
}
