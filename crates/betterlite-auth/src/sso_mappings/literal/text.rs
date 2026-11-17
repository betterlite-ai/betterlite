mod escape;

use super::{Cursor, Literal, LiteralError};

#[derive(Clone, Copy)]
enum TextPrefix {
    Text,
    RawText,
    Bytes,
    RawBytes,
}

struct TextStart {
    prefix: TextPrefix,
    quote_at: usize,
}

fn text_prefix(letters: &str) -> Option<TextPrefix> {
    match letters.to_lowercase().as_str() {
        "" | "u" => Some(TextPrefix::Text),
        "r" => Some(TextPrefix::RawText),
        "b" => Some(TextPrefix::Bytes),
        "br" | "rb" => Some(TextPrefix::RawBytes),
        _other => None,
    }
}

impl Cursor<'_> {
    pub(super) fn text_start(&self) -> Option<TextStart> {
        let rest = self.text.get(self.at..)?;
        let quote = rest.find(['\'', '"'])?;
        let prefix = rest.get(..quote)?;
        Some(TextStart {
            prefix: text_prefix(prefix)?,
            quote_at: self.at.saturating_add(quote),
        })
    }

    // implicit concatenation joins adjacent literals, but only within the expression's brackets
    pub(super) fn parse_text(&mut self, depth: usize) -> Result<Literal, LiteralError> {
        let Some(mut start) = self.text_start() else {
            return Err(LiteralError::Syntax);
        };
        let mut value = String::new();
        loop {
            let bytes = matches!(start.prefix, TextPrefix::Bytes | TextPrefix::RawBytes);
            let piece = self.scan_text(start)?;
            if !bytes {
                value.push_str(&piece);
            }
            if self.skip_trivia(depth)? {
                break;
            }
            let Some(next) = self.text_start() else {
                break;
            };
            let next_bytes = matches!(next.prefix, TextPrefix::Bytes | TextPrefix::RawBytes);
            if next_bytes != bytes {
                return Err(LiteralError::Syntax);
            }
            start = next;
        }
        if matches!(start.prefix, TextPrefix::Bytes | TextPrefix::RawBytes) {
            return Ok(Literal::Bytes);
        }
        Ok(Literal::Str(value))
    }

    fn scan_text(&mut self, start: TextStart) -> Result<String, LiteralError> {
        let Some(quote) = self.bytes.get(start.quote_at).copied() else {
            return Err(LiteralError::Syntax);
        };
        let triple = self.triple_quote(start.quote_at);
        let mut at = start.quote_at.saturating_add(if triple { 3 } else { 1 });
        let mut value = String::new();
        loop {
            let Some(character) = self.character(at) else {
                return Err(LiteralError::Syntax);
            };
            if self.closes_text(at, triple, quote) {
                self.at = at.saturating_add(if triple { 3 } else { 1 });
                return Ok(value);
            }
            if character == '\\' {
                at = self.scan_escape(at, start.prefix, &mut value)?;
                continue;
            }
            if !triple && matches!(character, '\n' | '\r') {
                return Err(LiteralError::Syntax);
            }
            if character == '\r' {
                value.push('\n');
                at = self.skip_newline_of(at.saturating_add(1));
                continue;
            }
            if matches!(start.prefix, TextPrefix::Bytes | TextPrefix::RawBytes)
                && !character.is_ascii()
            {
                return Err(LiteralError::Syntax);
            }
            value.push(character);
            at = at.saturating_add(character.len_utf8());
        }
    }

    fn triple_quote(&self, quote_at: usize) -> bool {
        let Some(quote) = self.bytes.get(quote_at).copied() else {
            return false;
        };
        if self.bytes.get(quote_at.saturating_add(1)).copied() != Some(quote) {
            return false;
        }
        self.bytes.get(quote_at.saturating_add(2)).copied() == Some(quote)
    }

    fn closes_text(&self, at: usize, triple: bool, quote: u8) -> bool {
        if self.bytes.get(at).copied() != Some(quote) {
            return false;
        }
        !triple || self.triple_quote(at)
    }
}
