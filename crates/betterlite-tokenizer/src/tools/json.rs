use std::ops::Range;

use serde::de::Error as _;

use crate::tools::error::ToolFormatError;

const WHITESPACE: &[u8] = b" \t\n\r";

// a cursor over tool JSON that serde already validated, so each keyword is read once in place
pub(super) struct Scanner<'json> {
    pub(super) bytes: &'json [u8],
    pub(super) at: usize,
}

impl Scanner<'_> {
    pub(super) fn peek(&mut self) -> Option<u8> {
        self.space();
        self.bytes.get(self.at).copied()
    }

    pub(super) fn take_byte(&mut self, expected: u8) -> Result<(), ToolFormatError> {
        self.space();
        match self.bytes.get(self.at).copied() {
            Some(actual) if actual == expected => {
                self.at = self.at.saturating_add(1);
                Ok(())
            }
            _ => Err(broken()),
        }
    }

    pub(super) fn skip_value(&mut self) -> Result<Range<usize>, ToolFormatError> {
        self.space();
        let start = self.at;
        match self.bytes.get(start).copied() {
            Some(b'"') => self.string_range(),
            Some(b'{') => self.skip_object(),
            Some(b'[') => self.skip_array(),
            Some(b't' | b'f' | b'n') => self.skip_word(),
            Some(_) => self.skip_number(),
            None => Err(broken()),
        }
    }

    pub(super) fn parse_string(&mut self, field: &'static str) -> Result<String, ToolFormatError> {
        let range = self.string_range()?;
        let slice = self.bytes.get(range).ok_or_else(broken)?;
        serde_json::from_slice(slice).map_err(|source| ToolFormatError::Nested { field, source })
    }

    fn space(&mut self) {
        while let Some(byte) = self.bytes.get(self.at).copied() {
            if !WHITESPACE.contains(&byte) {
                return;
            }
            self.at = self.at.saturating_add(1);
        }
    }

    fn string_range(&mut self) -> Result<Range<usize>, ToolFormatError> {
        let start = self.at;
        let mut index = start.saturating_add(1);
        loop {
            match self.bytes.get(index).copied() {
                Some(b'\\') => index = index.saturating_add(2),
                Some(b'"') => {
                    let end = index.saturating_add(1);
                    self.at = end;
                    return Ok(start..end);
                }
                Some(_) => index = index.saturating_add(1),
                None => return Err(broken()),
            }
        }
    }

    fn skip_object(&mut self) -> Result<Range<usize>, ToolFormatError> {
        let start = self.at;
        self.take_byte(b'{')?;
        if self.peek() == Some(b'}') {
            self.take_byte(b'}')?;
            return Ok(start..self.at);
        }
        loop {
            if self.peek() != Some(b'"') {
                return Err(broken());
            }
            self.string_range()?;
            self.take_byte(b':')?;
            self.skip_value()?;
            match self.peek() {
                Some(b',') => self.take_byte(b',')?,
                Some(b'}') => {
                    self.take_byte(b'}')?;
                    return Ok(start..self.at);
                }
                Some(_) | None => return Err(broken()),
            }
        }
    }

    fn skip_array(&mut self) -> Result<Range<usize>, ToolFormatError> {
        let start = self.at;
        self.take_byte(b'[')?;
        if self.peek() == Some(b']') {
            self.take_byte(b']')?;
            return Ok(start..self.at);
        }
        loop {
            self.skip_value()?;
            match self.peek() {
                Some(b',') => self.take_byte(b',')?,
                Some(b']') => {
                    self.take_byte(b']')?;
                    return Ok(start..self.at);
                }
                Some(_) | None => return Err(broken()),
            }
        }
    }

    fn skip_word(&mut self) -> Result<Range<usize>, ToolFormatError> {
        let start = self.at;
        while let Some(byte) = self.bytes.get(self.at).copied() {
            if !byte.is_ascii_alphabetic() {
                break;
            }
            self.at = self.at.saturating_add(1);
        }
        if self.at == start {
            return Err(broken());
        }
        Ok(start..self.at)
    }

    fn skip_number(&mut self) -> Result<Range<usize>, ToolFormatError> {
        let start = self.at;
        while let Some(byte) = self.bytes.get(self.at).copied() {
            if !matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                break;
            }
            self.at = self.at.saturating_add(1);
        }
        if self.at == start {
            return Err(broken());
        }
        Ok(start..self.at)
    }
}

// counts the deepest bracket nesting outside strings, so serde's own limit is never reached
pub(super) fn max_depth(bytes: &[u8]) -> usize {
    let mut depth = 0_usize;
    let mut deepest = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in bytes {
        if in_string {
            let closes = !escaped && byte == b'"';
            let starts_escape = !escaped && byte == b'\\';
            escaped = starts_escape;
            if closes {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth = depth.saturating_add(1);
                deepest = deepest.max(depth);
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

#[derive(Debug)]
pub(super) enum Field<T> {
    Value(T),
    Wrong(&'static str),
}

impl<T> Field<T> {
    pub(super) fn value(&self, field: &'static str, expected: &str) -> Result<&T, ToolFormatError> {
        match self {
            Self::Value(value) => Ok(value),
            Self::Wrong(found) => Err(mismatch(field, found, expected)),
        }
    }

    pub(super) fn shape(&self, index: usize, field: &'static str) -> Result<&T, ToolFormatError> {
        match self {
            Self::Value(value) => Ok(value),
            Self::Wrong(found) => Err(shape_mismatch(index, field, found, "a string")),
        }
    }
}

pub(super) fn text(
    source: &[u8],
    range: &Range<usize>,
    field: &'static str,
) -> Result<String, ToolFormatError> {
    let slice = source.get(range.clone()).ok_or_else(broken)?;
    serde_json::from_slice(slice).map_err(|failure| ToolFormatError::Nested {
        field,
        source: failure,
    })
}

pub(super) fn broken() -> ToolFormatError {
    ToolFormatError::Json {
        source: serde_json::Error::custom("the tool JSON could not be scanned"),
    }
}

pub(super) const fn kind_name(byte: u8) -> &'static str {
    match byte {
        b'{' => "an object",
        b'[' => "an array",
        b'"' => "a string",
        b't' | b'f' => "a boolean",
        b'n' => "null",
        _ => "a number",
    }
}

pub(super) fn mismatch(field: &'static str, found: &str, expected: &str) -> ToolFormatError {
    ToolFormatError::Nested {
        field,
        source: serde_json::Error::custom(format!("invalid type: {found}, expected {expected}")),
    }
}

pub(super) fn shape_mismatch(
    index: usize,
    field: &'static str,
    found: &str,
    expected: &str,
) -> ToolFormatError {
    ToolFormatError::Shape {
        index,
        field,
        source: serde_json::Error::custom(format!("invalid type: {found}, expected {expected}")),
    }
}
