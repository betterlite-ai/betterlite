mod tests;

use crate::sandbox_backend::protocol::{Encoding, FileData, ReadResult, ReadWindow};

pub const READ_LIMIT_LINES: usize = 2000;

// the boundaries Python's str.splitlines breaks on; "\r\n" counts as one
const LINE_BREAKS: [char; 10] = [
    '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}',
];

#[must_use]
pub fn slice_read(content: String, offset: usize, limit: usize) -> ReadResult {
    if content.trim().is_empty() {
        return ReadResult::Whole(FileData {
            content,
            encoding: Encoding::Utf8,
        });
    }
    if limit == 0 {
        return ReadResult::NoLinesRequested;
    }
    let lines = split_lines(&content);
    let total_lines = lines.len();
    if offset >= total_lines {
        return ReadResult::Error(format!(
            "Line offset {offset} exceeds file length ({total_lines} lines)"
        ));
    }
    let end_line = offset.saturating_add(limit).min(total_lines);
    let sliced = lines
        .get(offset..end_line)
        .unwrap_or_default()
        .concat()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    ReadResult::Window {
        file_data: FileData {
            content: sliced,
            encoding: Encoding::Utf8,
        },
        window: ReadWindow {
            total_lines,
            start_line: offset.saturating_add(1),
            end_line,
            next_offset: (end_line < total_lines).then_some(end_line),
        },
    }
}

fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = text;
    while let Some((index, found)) = rest
        .char_indices()
        .find(|(_, character)| LINE_BREAKS.contains(character))
    {
        let after = index.saturating_add(found.len_utf8());
        let crlf = found == '\r' && rest.get(after..).is_some_and(|tail| tail.starts_with('\n'));
        let end = if crlf { after.saturating_add(1) } else { after };
        let Some((line, tail)) = rest.split_at_checked(end) else {
            break;
        };
        lines.push(line);
        rest = tail;
    }
    if !rest.is_empty() {
        lines.push(rest);
    }
    lines
}
