use std::path::Path;

use backtrace::{Backtrace, BacktraceFrame, SymbolName};

pub const MAX_FRAMES: usize = 12;

// The workspace crates directory plays the role of LiteLLM's package directory
const CRATES_DIR: &str = "crates";

pub struct FrameSymbol<'symbol> {
    pub file: Option<&'symbol Path>,
    pub line: Option<u32>,
    pub name: Option<SymbolName<'symbol>>,
}

#[must_use]
pub fn litellm_frames(backtrace: &Backtrace) -> Vec<String> {
    let symbols = backtrace
        .frames()
        .iter()
        .flat_map(BacktraceFrame::symbols)
        .map(|symbol| FrameSymbol {
            file: symbol.filename(),
            line: symbol.lineno(),
            name: symbol.name(),
        });
    format_frames(symbols)
}

// Symbols come innermost first, the order a backtrace lists them in
#[must_use]
pub fn format_frames<'symbol, I>(symbols: I) -> Vec<String>
where
    I: DoubleEndedIterator<Item = FrameSymbol<'symbol>>,
{
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let crates_dir = manifest_dir.parent().unwrap_or(manifest_dir);
    let mut frames: Vec<String> = symbols
        .rev()
        .filter_map(|symbol| format_frame(symbol, crates_dir))
        .collect();
    let excess = frames.len().saturating_sub(MAX_FRAMES);
    frames.drain(..excess);
    frames
}

fn format_frame(symbol: FrameSymbol<'_>, crates_dir: &Path) -> Option<String> {
    let file = symbol.file?;
    let relative = file
        .strip_prefix(crates_dir)
        .or_else(|_| file.strip_prefix(CRATES_DIR))
        .ok()?;
    let line = symbol.line?;
    let name = symbol.name?;
    Some(format!(
        "{CRATES_DIR}/{}:{line} in {name:#}",
        relative.to_str()?
    ))
}
