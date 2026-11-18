use std::collections::BTreeMap;
use std::str::from_utf8;

use super::{DecodeError, JsonValue, MAX_MESSAGE_DEPTH, TypedValue};

pub(super) fn parse_object(text: &str) -> Result<TypedValue, DecodeError> {
    let mut parser = Parser {
        rest: text.as_bytes(),
        too_deep: false,
    };
    parser.skip_whitespace();
    if parser.rest.first().copied() != Some(b'{') {
        return Err(DecodeError::NotAnObject);
    }
    let members = parser.object(1).ok_or_else(|| parser.failure())?;
    parser.skip_whitespace();
    if !parser.rest.is_empty() {
        return Err(parser.failure());
    }
    Ok(members)
}

struct Parser<'text> {
    rest: &'text [u8],
    too_deep: bool,
}

impl Parser<'_> {
    fn failure(&self) -> DecodeError {
        if self.too_deep {
            DecodeError::TooDeep {
                limit: MAX_MESSAGE_DEPTH,
            }
        } else {
            DecodeError::Syntax
        }
    }

    fn next_unit(&mut self) -> Option<u8> {
        let (unit, rest) = self.rest.split_first()?;
        self.rest = rest;
        Some(*unit)
    }

    fn skip_whitespace(&mut self) {
        while matches!(
            self.rest.first().copied(),
            Some(b' ' | b'\t' | b'\n' | b'\r')
        ) {
            _ = self.next_unit();
        }
    }

    fn value(&mut self, depth: usize) -> Option<JsonValue> {
        match self.rest.first().copied()? {
            b'"' => self.string().map(JsonValue::Text),
            b'{' => self.object(depth).map(JsonValue::Object),
            b'[' => self.list(depth).map(JsonValue::List),
            b't' => self.literal(b"true", JsonValue::Bool(true)),
            b'f' => self.literal(b"false", JsonValue::Bool(false)),
            b'n' => self.literal(b"null", JsonValue::Null),
            b'N' => self.literal(b"NaN", JsonValue::Number(f64::NAN)),
            b'I' => self.literal(b"Infinity", JsonValue::Number(f64::INFINITY)),
            b'-' => self.signed(),
            b'0'..=b'9' => self.number().map(JsonValue::Number),
            _ => None,
        }
    }

    fn object(&mut self, depth: usize) -> Option<TypedValue> {
        if depth > MAX_MESSAGE_DEPTH {
            self.too_deep = true;
            return None;
        }
        if self.next_unit()? != b'{' {
            return None;
        }
        let mut members = BTreeMap::new();
        self.skip_whitespace();
        if self.rest.first().copied() == Some(b'}') {
            _ = self.next_unit()?;
            return Some(members);
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            if self.next_unit()? != b':' {
                return None;
            }
            self.skip_whitespace();
            let value = self.value(depth.saturating_add(1))?;
            members.insert(key, value);
            self.skip_whitespace();
            match self.next_unit()? {
                b',' => {}
                b'}' => return Some(members),
                _ => return None,
            }
        }
    }

    fn list(&mut self, depth: usize) -> Option<Vec<JsonValue>> {
        if depth > MAX_MESSAGE_DEPTH {
            self.too_deep = true;
            return None;
        }
        if self.next_unit()? != b'[' {
            return None;
        }
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.rest.first().copied() == Some(b']') {
            _ = self.next_unit()?;
            return Some(items);
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth.saturating_add(1))?);
            self.skip_whitespace();
            match self.next_unit()? {
                b',' => {}
                b']' => return Some(items),
                _ => return None,
            }
        }
    }

    fn literal(&mut self, expected: &[u8], value: JsonValue) -> Option<JsonValue> {
        self.rest = self.rest.strip_prefix(expected)?;
        Some(value)
    }

    fn signed(&mut self) -> Option<JsonValue> {
        if let Some(rest) = self.rest.strip_prefix(b"-Infinity") {
            self.rest = rest;
            return Some(JsonValue::Number(f64::NEG_INFINITY));
        }
        self.number().map(JsonValue::Number)
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.rest;
        self.integer()?;
        if self.rest.starts_with(b".") {
            self.rest = self.rest.get(1..)?;
            self.digits()?;
        }
        if matches!(self.rest.first().copied(), Some(b'e' | b'E')) {
            self.rest = self.rest.get(1..)?;
            if matches!(self.rest.first().copied(), Some(b'+' | b'-')) {
                self.rest = self.rest.get(1..)?;
            }
            self.digits()?;
        }
        let consumed = start.len().saturating_sub(self.rest.len());
        let token = from_utf8(start.get(..consumed)?).ok()?;
        token.parse::<f64>().ok()
    }

    fn integer(&mut self) -> Option<()> {
        if self.rest.first().copied() == Some(b'-') {
            _ = self.next_unit()?;
        }
        match self.next_unit()? {
            b'0' => Some(()),
            b'1'..=b'9' => {
                self.digits_optional();
                Some(())
            }
            _ => None,
        }
    }

    fn digits_optional(&mut self) {
        while matches!(self.rest.first().copied(), Some(b'0'..=b'9')) {
            _ = self.next_unit();
        }
    }

    fn digits(&mut self) -> Option<()> {
        let before = self.rest.len();
        self.digits_optional();
        (self.rest.len() < before).then_some(())
    }

    fn string(&mut self) -> Option<String> {
        if self.next_unit()? != b'"' {
            return None;
        }
        let mut text = Vec::new();
        loop {
            let rest = self.rest;
            if rest.starts_with(b"\"") {
                self.rest = rest.get(1..)?;
                return String::from_utf8(text).ok();
            }
            if rest.starts_with(b"\\") {
                self.rest = rest.get(1..)?;
                self.escape(&mut text)?;
                continue;
            }
            let unit = rest.first().copied()?;
            if unit < 0x20 {
                return None;
            }
            text.push(unit);
            self.rest = rest.get(1..)?;
        }
    }

    fn escape(&mut self, text: &mut Vec<u8>) -> Option<()> {
        let code = match self.next_unit()? {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => self.unicode()?,
            _ => return None,
        };
        let mut buffer = [0_u8; 4];
        text.extend_from_slice(code.encode_utf8(&mut buffer).as_bytes());
        Some(())
    }

    fn unicode(&mut self) -> Option<char> {
        let value = self.hex4()?;
        if let Some(character) = char::from_u32(value) {
            return Some(character);
        }
        let high = value.checked_sub(0xD800)?;
        if high >= 0x400 {
            return None;
        }
        let rest = self.rest.strip_prefix(b"\\u")?;
        self.rest = rest;
        let low = self.hex4()?;
        let low_offset = low.checked_sub(0xDC00)?;
        if low_offset >= 0x400 {
            return None;
        }
        let shifted = high.checked_shl(10)?;
        let combined = 0x1_0000_u32.checked_add(shifted)?.checked_add(low_offset)?;
        char::from_u32(combined)
    }

    fn hex4(&mut self) -> Option<u32> {
        let mut value = 0_u32;
        for _ in 0..4 {
            let unit = self.next_unit()?;
            value = value.saturating_mul(16).saturating_add(hex_digit(unit)?);
        }
        Some(value)
    }
}

fn hex_digit(unit: u8) -> Option<u32> {
    let digit = u32::from(unit);
    match unit {
        b'0'..=b'9' => digit.checked_sub(u32::from(b'0')),
        b'a'..=b'f' => digit
            .checked_sub(u32::from(b'a'))
            .map(|offset| offset.saturating_add(10)),
        b'A'..=b'F' => digit
            .checked_sub(u32::from(b'A'))
            .map(|offset| offset.saturating_add(10)),
        _ => None,
    }
}
