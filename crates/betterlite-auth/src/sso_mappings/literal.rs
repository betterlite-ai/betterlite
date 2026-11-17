mod brace;
pub mod hashable;
mod number;
mod scanner;
mod text;

use std::str;

use thiserror::Error;

pub const MAX_LITERAL_DEPTH: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Literal {
    None,
    Bool(bool),
    Number,
    Ellipsis,
    Bytes,
    Str(String),
    List(Vec<Self>),
    Tuple(Vec<Self>),
    Set(Vec<Self>),
    Dict(Vec<(Self, Self)>),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LiteralError {
    #[error("the literal is not UTF-8 text")]
    NotUtf8,
    #[error("the literal has a syntax error")]
    Syntax,
    #[error("the literal is not a Python literal")]
    NotLiteral,
    #[error("the literal nests deeper than {limit} levels")]
    TooDeep { limit: usize },
    #[error("a \\N{{...}} escape needs the Unicode name table, which betterlite does not carry")]
    NamedEscape,
}

// Python compiles before evaluating, so a syntax error wins over has_unhashable_member's TypeError
pub fn parse_literal(bytes: &[u8]) -> Result<Literal, LiteralError> {
    let Ok(text) = str::from_utf8(bytes) else {
        return Err(LiteralError::NotUtf8);
    };
    let mut cursor = Cursor {
        text,
        bytes: text.as_bytes(),
        at: 0,
    };
    cursor.parse_document()
}

struct Cursor<'text> {
    text: &'text str,
    bytes: &'text [u8],
    at: usize,
}

// a parenthesized group is a tuple unless it holds exactly one element, which Python reads as that element
#[derive(Clone, Copy, PartialEq, Eq)]
enum Brackets {
    List,
    Tuple,
}

impl Brackets {
    const fn close(self) -> u8 {
        match self {
            Self::List => b']',
            Self::Tuple => b')',
        }
    }

    const fn collect(self, items: Vec<Literal>) -> Literal {
        match self {
            Self::List => Literal::List(items),
            Self::Tuple => Literal::Tuple(items),
        }
    }
}

impl Cursor<'_> {
    fn parse_document(&mut self) -> Result<Literal, LiteralError> {
        scanner::skip_trivia(self, 0)?;
        let value = self.parse_value(0)?;
        scanner::skip_trivia(self, 0)?;
        if self.bytes.get(self.at).is_some() {
            return Err(LiteralError::Syntax);
        }
        Ok(value)
    }

    fn parse_value(&mut self, depth: usize) -> Result<Literal, LiteralError> {
        scanner::skip_trivia(self, depth)?;
        let Some(byte) = self.bytes.get(self.at).copied() else {
            return Err(LiteralError::Syntax);
        };
        match byte {
            b'{' => brace::parse_brace(self, depth),
            b'[' => self.parse_sequence(depth, Brackets::List),
            b'(' => self.parse_sequence(depth, Brackets::Tuple),
            b'+' | b'-' => self.parse_signed(depth),
            _other => self.parse_scalar(depth),
        }
    }

    fn parse_scalar(&mut self, depth: usize) -> Result<Literal, LiteralError> {
        if text::text_start(self).is_some() {
            return text::parse_text(self, depth);
        }
        if number::number_start(self) {
            return number::parse_number(self);
        }
        if self.take_ellipsis() {
            return Ok(Literal::Ellipsis);
        }
        self.parse_word()
    }

    fn parse_signed(&mut self, depth: usize) -> Result<Literal, LiteralError> {
        self.at = self.at.saturating_add(1);
        if scanner::skip_trivia(self, depth)? {
            return Err(LiteralError::Syntax);
        }
        if self.bytes.get(self.at).copied() == Some(b'(') {
            let parenthesized = self.parse_sequence(depth, Brackets::Tuple)?;
            if matches!(parenthesized, Literal::Number) {
                return Ok(parenthesized);
            }
            return Err(LiteralError::NotLiteral);
        }
        if !number::number_start(self) {
            return Err(LiteralError::NotLiteral);
        }
        number::parse_number(self)
    }

    fn parse_sequence(
        &mut self,
        depth: usize,
        brackets: Brackets,
    ) -> Result<Literal, LiteralError> {
        if scanner::open(self, depth, brackets.close())? {
            return Ok(brackets.collect(Vec::new()));
        }
        let first = self.first_element(depth)?;
        if brackets == Brackets::Tuple && self.take(brackets.close()) {
            return Ok(first);
        }
        let items = self.parse_rest(depth, brackets.close(), vec![first])?;
        Ok(brackets.collect(items))
    }

    // the first element of a bracketed sequence, with the trivia after it skipped
    fn first_element(&mut self, depth: usize) -> Result<Literal, LiteralError> {
        if self.bytes.get(self.at).copied() == Some(b'*') {
            return Err(LiteralError::NotLiteral);
        }
        let first = self.parse_value(depth.saturating_add(1))?;
        scanner::skip_trivia(self, depth.saturating_add(1))?;
        Ok(first)
    }

    fn parse_rest(
        &mut self,
        depth: usize,
        close: u8,
        mut items: Vec<Literal>,
    ) -> Result<Vec<Literal>, LiteralError> {
        loop {
            if !scanner::close_or_comma(self, depth, close)? {
                return Ok(items);
            }
            if self.bytes.get(self.at).copied() == Some(b'*') {
                return Err(LiteralError::NotLiteral);
            }
            items.push(self.parse_value(depth.saturating_add(1))?);
        }
    }

    fn take_ellipsis(&mut self) -> bool {
        let Some(rest) = self.text.get(self.at..) else {
            return false;
        };
        if !rest.starts_with("...") {
            return false;
        }
        self.at = self.at.saturating_add(3);
        true
    }

    fn parse_word(&mut self) -> Result<Literal, LiteralError> {
        let start = self.at;
        while let Some(byte) = self.bytes.get(self.at).copied() {
            if !byte.is_ascii_alphanumeric() && byte != b'_' {
                break;
            }
            self.at = self.at.saturating_add(1);
        }
        let Some(word) = self.bytes.get(start..self.at) else {
            return Err(LiteralError::Syntax);
        };
        match word {
            b"True" => Ok(Literal::Bool(true)),
            b"False" => Ok(Literal::Bool(false)),
            b"None" => Ok(Literal::None),
            b"set" => self.parse_empty_set(),
            _other => Err(LiteralError::NotLiteral),
        }
    }

    fn parse_empty_set(&mut self) -> Result<Literal, LiteralError> {
        scanner::skip_trivia(self, 1)?;
        if !self.take(b'(') {
            return Err(LiteralError::NotLiteral);
        }
        scanner::skip_trivia(self, 1)?;
        if !self.take(b')') {
            return Err(LiteralError::NotLiteral);
        }
        Ok(Literal::Set(Vec::new()))
    }

    fn skip_newline_of(&self, at: usize) -> usize {
        if self.bytes.get(at).copied() == Some(b'\n') {
            return at.saturating_add(1);
        }
        at
    }

    fn character(&self, at: usize) -> Option<char> {
        if let Some(rest) = self.text.get(at..) {
            return rest.chars().next();
        }
        None
    }

    fn take(&mut self, expected: u8) -> bool {
        if self.bytes.get(self.at).copied() != Some(expected) {
            return false;
        }
        self.at = self.at.saturating_add(1);
        true
    }
}
