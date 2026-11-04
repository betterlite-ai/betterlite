use std::collections::BTreeSet;
use std::io::{self, Write};

use crate::terminal_styling::{blue, bold, green, grey};

use super::PROVIDERS;
use super::raw_mode::TtyRawMode;

const CURSOR_HIDE: &str = "\x1b[?25l";
const CURSOR_SHOW: &str = "\x1b[?25h";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Space,
    Enter,
    Cancel,
    Other,
}

pub trait KeySource {
    fn next_key(&mut self) -> io::Result<Key>;
}

pub trait ByteSource {
    fn next_byte(&mut self) -> io::Result<u8>;
}

pub struct TtyKeySource<S: ByteSource = TtyRawMode> {
    pub(crate) bytes: S,
}

impl<S: ByteSource> KeySource for TtyKeySource<S> {
    fn next_key(&mut self) -> io::Result<Key> {
        match self.bytes.next_byte()? {
            0x03 | 0x04 => Ok(Key::Cancel),
            b' ' => Ok(Key::Space),
            b'\r' | b'\n' => Ok(Key::Enter),
            0x1b => {
                if self.bytes.next_byte()? != b'[' {
                    return Ok(Key::Other);
                }
                match self.bytes.next_byte()? {
                    b'A' => Ok(Key::Up),
                    b'B' => Ok(Key::Down),
                    _ => Ok(Key::Other),
                }
            }
            _ => Ok(Key::Other),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InteractiveError {
    #[error("provider selection was cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] io::Error),
}

struct CursorVisibilityGuard<'writer> {
    writer: &'writer mut dyn Write,
    active: bool,
}

impl Drop for CursorVisibilityGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            drop(write!(self.writer, "{CURSOR_SHOW}"));
            drop(self.writer.flush());
        }
    }
}

pub fn select_interactive<K>(
    key_source: &mut K,
    writer: &mut dyn Write,
    supports_color: bool,
) -> Result<BTreeSet<usize>, InteractiveError>
where
    K: KeySource,
{
    if supports_color {
        write!(writer, "{CURSOR_HIDE}")?;
        writer.flush()?;
    }
    let guard = CursorVisibilityGuard {
        writer,
        active: supports_color,
    };
    run_selection_loop(key_source, guard.writer, supports_color)
}

enum LoopOutcome {
    Continue,
    Finished(BTreeSet<usize>),
    Cancelled,
}

fn handle_key(key: Key, cursor: &mut usize, selected: &mut BTreeSet<usize>) -> LoopOutcome {
    match key {
        Key::Up => {
            *cursor = cursor
                .checked_sub(1)
                .unwrap_or(PROVIDERS.len().saturating_sub(1));
            LoopOutcome::Continue
        }
        Key::Down => {
            *cursor = cursor
                .checked_add(1)
                .filter(|next| *next < PROVIDERS.len())
                .unwrap_or(0);
            LoopOutcome::Continue
        }
        Key::Space => {
            if !selected.remove(cursor) {
                selected.insert(*cursor);
            }
            LoopOutcome::Continue
        }
        Key::Enter => {
            if selected.is_empty() {
                selected.insert(*cursor);
            }
            LoopOutcome::Finished(selected.clone())
        }
        Key::Cancel => LoopOutcome::Cancelled,
        Key::Other => LoopOutcome::Continue,
    }
}

fn run_selection_loop<K>(
    key_source: &mut K,
    writer: &mut dyn Write,
    supports_color: bool,
) -> Result<BTreeSet<usize>, InteractiveError>
where
    K: KeySource,
{
    let mut cursor = 0_usize;
    let mut selected: BTreeSet<usize> = BTreeSet::new();
    let mut first_render = true;
    loop {
        let render_state = RenderState {
            first_render,
            supports_color,
        };
        write!(
            writer,
            "{}",
            render_selector(cursor, &selected, &render_state)
        )?;
        writer.flush()?;
        first_render = false;
        match handle_key(key_source.next_key()?, &mut cursor, &mut selected) {
            LoopOutcome::Continue => {}
            LoopOutcome::Finished(result) => return Ok(result),
            LoopOutcome::Cancelled => return Err(InteractiveError::Cancelled),
        }
    }
}

struct RenderState {
    first_render: bool,
    supports_color: bool,
}

fn render_selector(cursor: usize, selected: &BTreeSet<usize>, state: &RenderState) -> String {
    let supports_color = state.supports_color;
    let header = format!("\n  {}\n", bold("Add your first model", supports_color));
    let mut content = header;
    content.push_str(&grey(
        "  \u{2191}\u{2193} to navigate \u{b7} Space to select \u{b7} Enter to confirm",
        supports_color,
    ));
    content.push_str("\n\n");
    for (index, provider) in PROVIDERS.iter().enumerate() {
        let arrow = if index == cursor {
            blue("\u{276f}", supports_color)
        } else {
            " ".to_owned()
        };
        let bullet = if selected.contains(&index) {
            green("\u{25c9}", supports_color)
        } else {
            grey("\u{25cb}", supports_color)
        };
        let name = if index == cursor {
            bold(provider.name, supports_color)
        } else {
            provider.name.to_owned()
        };
        let description = grey(provider.description, supports_color);
        let line = format!("  {arrow} {bullet} {name}  {description}\n");
        content.push_str(&line);
    }
    content.push('\n');

    if !state.first_render && supports_color {
        format!("\x1b[{}A{content}", content.matches('\n').count())
    } else {
        content
    }
}

mod tests;
