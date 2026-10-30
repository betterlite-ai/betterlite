use std::collections::BTreeMap;
use std::io::Error as IoError;

use super::{JsonValue, MAX_STORED_BYTES, MAX_STORED_DEPTH};

#[derive(Debug, thiserror::Error)]
pub enum McpStoredDecodeError {
    #[error("stored MCP JSON exceeds {MAX_STORED_BYTES} bytes")]
    TooLarge,
    #[error("stored MCP JSON exceeds depth {MAX_STORED_DEPTH}")]
    TooDeep,
    #[error("stored MCP JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn decode_mcp_stored_contract(bytes: &[u8]) -> Result<JsonValue, McpStoredDecodeError> {
    if bytes.len() > MAX_STORED_BYTES {
        return Err(McpStoredDecodeError::TooLarge);
    }
    check_depth(bytes)?;
    let mut parser = Parser { bytes, position: 0 };
    let value = parser.parse_value()?;
    parser.skip_whitespace();
    if parser.position != bytes.len() {
        return Err(McpStoredDecodeError::Json(invalid_json()));
    }
    Ok(value)
}

fn check_depth(bytes: &[u8]) -> Result<(), McpStoredDecodeError> {
    let mut quoted = false;
    let mut escaped = false;
    let mut depth = 0_u8;
    for &byte in bytes {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' if quoted => escaped = true,
            b'"' => quoted = !quoted,
            b'[' | b'{' if !quoted => {
                depth = depth.saturating_add(1);
                if usize::from(depth) > MAX_STORED_DEPTH {
                    return Err(McpStoredDecodeError::TooDeep);
                }
            }
            b']' | b'}' if !quoted => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

struct Parser<'input> {
    bytes: &'input [u8],
    position: usize,
}

impl Parser<'_> {
    fn parse_value(&mut self) -> Result<JsonValue, serde_json::Error> {
        let byte = self.next_byte()?;
        match byte {
            b'n' => self.parse_fixed(b"null", JsonValue::Null),
            b't' => self.parse_fixed(b"true", JsonValue::Bool(true)),
            b'f' => self.parse_fixed(b"false", JsonValue::Bool(false)),
            b'"' => self.parse_string().map(JsonValue::String),
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(invalid_json()),
        }
    }

    fn parse_fixed(
        &mut self,
        literal: &[u8],
        value: JsonValue,
    ) -> Result<JsonValue, serde_json::Error> {
        let end = self.position.saturating_add(literal.len());
        if self.bytes.get(self.position..end) != Some(literal) {
            return Err(invalid_json());
        }
        self.position = end;
        Ok(value)
    }

    fn next_byte(&mut self) -> Result<u8, serde_json::Error> {
        self.skip_whitespace();
        self.bytes
            .get(self.position)
            .copied()
            .ok_or_else(invalid_json)
    }

    fn parse_string(&mut self) -> Result<String, serde_json::Error> {
        let start = self.position;
        self.position = self.position.saturating_add(1);
        self.scan_string_end()?;
        self.decode_string(start)
    }

    fn scan_string_end(&mut self) -> Result<(), serde_json::Error> {
        let mut escaped = false;
        while let Some(&byte) = self.bytes.get(self.position) {
            self.position = self.position.saturating_add(1);
            if escaped {
                escaped = false;
                continue;
            }
            if byte == b'\\' {
                escaped = true;
                continue;
            }
            if byte == b'"' {
                return Ok(());
            }
        }
        Err(invalid_json())
    }

    fn decode_string(&self, start: usize) -> Result<String, serde_json::Error> {
        let Some(text) = self.bytes.get(start..self.position) else {
            return Err(invalid_json());
        };
        serde_json::from_slice(text)
    }

    fn parse_number(&mut self) -> Result<JsonValue, serde_json::Error> {
        let text = self.scan_number()?;
        serde_json::from_slice(text).map(JsonValue::Number)
    }

    fn scan_number(&mut self) -> Result<&[u8], serde_json::Error> {
        let start = self.position;
        while let Some(&byte) = self.bytes.get(self.position) {
            if !matches!(byte, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                break;
            }
            self.position = self.position.saturating_add(1);
        }
        let Some(text) = self.bytes.get(start..self.position) else {
            return Err(invalid_json());
        };
        Ok(text)
    }

    fn parse_array(&mut self) -> Result<JsonValue, serde_json::Error> {
        self.position = self.position.saturating_add(1);
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.take(b']') {
            return Ok(JsonValue::Array(values));
        }
        loop {
            values.push(self.parse_value()?);
            self.skip_whitespace();
            if self.take(b']') {
                return Ok(JsonValue::Array(values));
            }
            if !self.take(b',') {
                return Err(invalid_json());
            }
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, serde_json::Error> {
        self.position = self.position.saturating_add(1);
        let mut values = BTreeMap::new();
        self.skip_whitespace();
        if self.take(b'}') {
            return Ok(JsonValue::Object(values));
        }
        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            if !self.take(b':') {
                return Err(invalid_json());
            }
            values.insert(key, self.parse_value()?);
            self.skip_whitespace();
            if self.take(b'}') {
                return Ok(JsonValue::Object(values));
            }
            if !self.take(b',') {
                return Err(invalid_json());
            }
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(
            self.bytes.get(self.position),
            Some(b' ' | b'\n' | b'\r' | b'\t')
        ) {
            self.position = self.position.saturating_add(1);
        }
    }

    fn take(&mut self, expected: u8) -> bool {
        if self.bytes.get(self.position) == Some(&expected) {
            self.position = self.position.saturating_add(1);
            true
        } else {
            false
        }
    }
}

fn invalid_json() -> serde_json::Error {
    match serde_json::from_slice::<bool>(b"") {
        Ok(_) => serde_json::Error::io(IoError::other("invalid JSON")),
        Err(error) => error,
    }
}
