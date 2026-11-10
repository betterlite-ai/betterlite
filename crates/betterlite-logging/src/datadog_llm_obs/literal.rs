use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::values::as_text;

mod numbers;
mod strings;

// Above this nesting the decoder would recurse past a gateway stack, so the literal is refused.
pub const MAX_LITERAL_DEPTH: usize = 128;

// Stands in for a value Python reads and JSON cannot carry, so the rest of the literal survives.
const PLACEHOLDER_LITERAL: &str = "<unrepresentable literal>";

#[must_use]
pub fn literal_json(text: &str) -> Option<Json> {
    let mut parser = Parser {
        text,
        bytes: text.as_bytes(),
        position: 0,
        depth: 0,
    };
    parser.skip(true);
    let value = parser.value()?;
    parser.skip(true);
    (parser.position == parser.bytes.len()).then_some(value)
}

struct Parser<'text> {
    text: &'text str,
    bytes: &'text [u8],
    position: usize,
    depth: usize,
}

enum Next {
    Element,
    Closing,
    TrailingComma,
}

impl Parser<'_> {
    fn value(&mut self) -> Option<Json> {
        self.skip(self.depth > 0);
        match self.bytes.get(self.position).copied()? {
            b'[' => self.sequence(b']', false),
            b'(' => self.sequence(b')', true),
            b'{' => self.braced(),
            b'\'' | b'"' | b'u' | b'U' => strings::group(self, false).map(Json::Text),
            b'r' | b'R' => {
                let second = self.bytes.get(self.position.saturating_add(1)).copied();
                if matches!(second, Some(b'b' | b'B')) {
                    self.bytes_value()
                } else {
                    strings::group(self, false).map(Json::Text)
                }
            }
            b'b' | b'B' => self.bytes_value(),
            b's' => self.empty_set(),
            b'+' | b'-' | b'.' | b'0'..=b'9' => numbers::number(self),
            b'T' => self.keyword("True", Json::Bool(true)),
            b'F' => self.keyword("False", Json::Bool(false)),
            b'N' => self.keyword("None", Json::Null),
            _ => None,
        }
    }

    fn bytes_value(&mut self) -> Option<Json> {
        strings::group(self, true)?;
        Some(Json::Text(PLACEHOLDER_LITERAL.to_owned()))
    }

    fn empty_set(&mut self) -> Option<Json> {
        let end = self.position.saturating_add(3);
        if self.bytes.get(self.position..end) != Some(b"set".as_slice()) {
            return None;
        }
        self.position = end;
        for (bracket, newlines) in [(b'(', self.depth > 0), (b')', true)] {
            self.skip(newlines);
            if self.bytes.get(self.position).copied()? != bracket {
                return None;
            }
            self.position = self.position.saturating_add(1);
        }
        Some(Json::List(Vec::new()))
    }

    fn keyword(&mut self, word: &str, value: Json) -> Option<Json> {
        let end = self.position.saturating_add(word.len());
        let following = self.bytes.get(end).copied();
        if self.bytes.get(self.position..end) != Some(word.as_bytes())
            || following.is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return None;
        }
        self.position = end;
        Some(value)
    }

    fn sequence(&mut self, close: u8, tuple: bool) -> Option<Json> {
        self.position = self.position.saturating_add(1);
        self.skip(true);
        if self.bytes.get(self.position).copied()? == close {
            self.position = self.position.saturating_add(1);
            return Some(Json::List(Vec::new()));
        }
        self.enter()?;
        let mut items = Vec::new();
        let separated = loop {
            items.push(self.value()?);
            match self.separator(close)? {
                Next::Element => {}
                Next::Closing => break false,
                Next::TrailingComma => break true,
            }
        };
        self.depth = self.depth.saturating_sub(1);
        if tuple && items.len() == 1 && !separated {
            return items.pop();
        }
        Some(Json::List(items))
    }

    fn braced(&mut self) -> Option<Json> {
        self.position = self.position.saturating_add(1);
        self.skip(true);
        if self.bytes.get(self.position).copied()? == b'}' {
            self.position = self.position.saturating_add(1);
            return Some(Json::Object(IndexMap::new()));
        }
        self.enter()?;
        let first = self.value()?;
        self.skip(true);
        if self.bytes.get(self.position).copied()? == b':' {
            let object = Json::Object(self.dictionary(first)?);
            self.depth = self.depth.saturating_sub(1);
            return Some(object);
        }
        let mut items = vec![first];
        while matches!(self.separator(b'}')?, Next::Element) {
            items.push(self.value()?);
        }
        self.depth = self.depth.saturating_sub(1);
        Some(Json::List(items))
    }

    fn dictionary(&mut self, first: Json) -> Option<IndexMap<String, Json>> {
        let mut fields = IndexMap::new();
        let mut key = first;
        loop {
            let name = as_text(&key).unwrap_or(PLACEHOLDER_LITERAL).to_owned();
            fields.insert(name, self.colon_value()?);
            if !matches!(self.separator(b'}')?, Next::Element) {
                return Some(fields);
            }
            key = self.value()?;
        }
    }

    fn colon_value(&mut self) -> Option<Json> {
        self.skip(true);
        if self.bytes.get(self.position).copied()? != b':' {
            return None;
        }
        self.position = self.position.saturating_add(1);
        self.value()
    }

    fn separator(&mut self, close: u8) -> Option<Next> {
        self.skip(true);
        let byte = self.bytes.get(self.position).copied()?;
        self.position = self.position.saturating_add(1);
        if byte == close {
            return Some(Next::Closing);
        }
        if byte != b',' {
            return None;
        }
        self.skip(true);
        if self.bytes.get(self.position).copied()? == close {
            self.position = self.position.saturating_add(1);
            return Some(Next::TrailingComma);
        }
        Some(Next::Element)
    }

    const fn enter(&mut self) -> Option<()> {
        if self.depth >= MAX_LITERAL_DEPTH {
            return None;
        }
        self.depth = self.depth.saturating_add(1);
        Some(())
    }

    // Newlines end the expression at the top level and are ignored inside brackets, as Python's tokenizer does.
    fn skip(&mut self, newlines: bool) {
        while let Some(byte) = self.bytes.get(self.position).copied() {
            match byte {
                b' ' | b'\t' | b'\x0c' => self.position = self.position.saturating_add(1),
                b'\n' | b'\r' if newlines => self.position = self.position.saturating_add(1),
                b'#' => {
                    let remaining = self.bytes.get(self.position..).unwrap_or_default();
                    let end = remaining.iter().position(|candidate| *candidate == b'\n');
                    self.position = self.position.saturating_add(end.unwrap_or(remaining.len()));
                }
                _ => return,
            }
        }
    }
}
