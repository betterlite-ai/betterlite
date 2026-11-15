use std::fmt::Display;

mod tests;

const MAX_ERROR_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location<'file> {
    pub file: &'file str,
    pub line: u32,
}

#[must_use]
pub fn detail<Message>(class: &str, location: Option<Location<'_>>, message: Message) -> String
where
    Message: Display,
{
    let place = location.map_or_else(String::new, |at| {
        format!(" at {}:{}", basename(at.file), at.line)
    });
    let full = format!("{class}{place}: {message}");
    full.chars().take(MAX_ERROR_CHARS).collect()
}

// Python keeps everything after the last slash, so a backslash is part of the name
fn basename(file: &str) -> &str {
    match file.rsplit_once('/') {
        Some((_, name)) => name,
        None => file,
    }
}
