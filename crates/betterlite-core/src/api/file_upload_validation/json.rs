use super::line_shape::LineShape;
// LiteLLM's json.loads recurses about a thousand levels and its handler does not catch RecursionError
const MAX_DEPTH: usize = 128;
pub(super) enum Scan {
    Valid,
    NotObject,
    Invalid,
    Missing(&'static str),
}
pub(super) fn scan(text: &[u8], shape: LineShape) -> Scan {
    let mut parser = Parser {
        rest: text,
        shape,
        seen: 0,
    };
    parser.skip_whitespace();
    let object = parser.peek() == Some(b'{');
    let parsed = if object {
        parser.object(0, true)
    } else {
        parser.value(0)
    };
    if parsed.is_none() {
        return Scan::Invalid;
    }
    parser.skip_whitespace();
    if !parser.rest.is_empty() {
        return Scan::Invalid;
    }
    if !object {
        return Scan::NotObject;
    }
    match shape.first_missing(parser.seen) {
        Some(key) => Scan::Missing(key),
        None => Scan::Valid,
    }
}
struct Parser<'a> {
    rest: &'a [u8],
    shape: LineShape,
    seen: u8,
}
impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        match self.rest.first() {
            Some(unit) => Some(*unit),
            None => None,
        }
    }
    fn next_unit(&mut self) -> Option<u8> {
        let (unit, rest) = self.rest.split_first()?;
        self.rest = rest;
        Some(*unit)
    }
    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            drop(self.next_unit());
        }
    }
    fn value(&mut self, depth: usize) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        match self.peek()? {
            b'"' => self.string(false).map(drop),
            b'{' => self.object(depth, false),
            b'[' => self.array(depth),
            b't' => self.literal(b"true"),
            b'f' => self.literal(b"false"),
            b'n' => self.literal(b"null"),
            b'N' => self.literal(b"NaN"),
            b'I' => self.literal(b"Infinity"),
            b'-' => self.signed(),
            b'0'..=b'9' => self.number(),
            _ => None,
        }
    }
    fn member(&mut self, depth: usize, track_keys: bool) -> Option<Option<String>> {
        let key = self.string(track_keys)?;
        self.skip_whitespace();
        if self.next_unit()? != b':' {
            return None;
        }
        self.skip_whitespace();
        self.value(depth)?;
        Some(key)
    }
    fn object(&mut self, depth: usize, track_keys: bool) -> Option<()> {
        if self.next_unit()? != b'{' {
            return None;
        }
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            drop(self.next_unit()?);
            return Some(());
        }
        loop {
            self.skip_whitespace();
            let key = self.member(depth.saturating_add(1), track_keys)?;
            if let Some(key) = key {
                self.mark(&key);
            }
            self.skip_whitespace();
            match self.next_unit()? {
                b',' => {}
                b'}' => return Some(()),
                _ => return None,
            }
        }
    }
    fn array(&mut self, depth: usize) -> Option<()> {
        drop(self.next_unit()?);
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            drop(self.next_unit()?);
            return Some(());
        }
        loop {
            self.skip_whitespace();
            self.value(depth.saturating_add(1))?;
            self.skip_whitespace();
            match self.next_unit()? {
                b',' => {}
                b']' => return Some(()),
                _ => return None,
            }
        }
    }
    fn signed(&mut self) -> Option<()> {
        if let Some(rest) = self.rest.strip_prefix(b"-Infinity") {
            self.rest = rest;
            return Some(());
        }
        self.number()
    }
    fn number(&mut self) -> Option<()> {
        self.integer()?;
        if self.peek() == Some(b'.') {
            drop(self.next_unit()?);
            self.digits()?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            drop(self.next_unit()?);
            if matches!(self.peek(), Some(b'+' | b'-')) {
                drop(self.next_unit()?);
            }
            self.digits()?;
        }
        Some(())
    }
    fn integer(&mut self) -> Option<()> {
        if self.peek() == Some(b'-') {
            drop(self.next_unit()?);
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
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            drop(self.next_unit());
        }
    }
    fn digits(&mut self) -> Option<()> {
        let before = self.rest.len();
        self.digits_optional();
        (self.rest.len() < before).then_some(())
    }
    fn literal(&mut self, expected: &[u8]) -> Option<()> {
        self.rest = self.rest.strip_prefix(expected)?;
        Some(())
    }
    fn mark(&mut self, key: &str) {
        if let Some(bit) = self.shape.key_bit(key) {
            self.seen = self.seen.bitor(bit);
        }
    }
    fn string(&mut self, capture: bool) -> Option<Option<String>> {
        if self.next_unit()? != b'"' {
            return None;
        }
        let mut text = String::new();
        let mut ascii = capture;
        loop {
            match self.next_unit()? {
                b'"' => return Some(ascii.then_some(text)),
                b'\\' => ascii = self.escape(&mut text, ascii)?,
                0x00..=0x1F => return None,
                unit @ 0x20..=0x7F => {
                    if ascii {
                        text.push(char::from(unit));
                    }
                }
                _ => ascii = false,
            }
        }
    }
    fn escape(&mut self, text: &mut String, ascii: bool) -> Option<bool> {
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
        let keep = ascii && code.is_ascii();
        if keep {
            text.push(code);
        }
        Some(keep)
    }
    fn unicode(&mut self) -> Option<char> {
        let hex = self.rest.get(..4)?;
        if !hex.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let value = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
        self.rest = self.rest.get(4..)?;
        Some(char::from_u32(value).unwrap_or(char::REPLACEMENT_CHARACTER))
    }
}
