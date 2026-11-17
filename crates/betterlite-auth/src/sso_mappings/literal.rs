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

impl Cursor<'_> {
    fn byte(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn parse_document(&mut self) -> Result<Literal, LiteralError> {
        self.skip_trivia(0)?;
        let value = self.parse_value(0)?;
        self.skip_trivia(0)?;
        if self.byte().is_some() {
            return Err(LiteralError::Syntax);
        }
        Ok(value)
    }

    fn parse_value(&mut self, depth: usize) -> Result<Literal, LiteralError> {
        self.skip_trivia(depth)?;
        match self.byte() {
            Some(b'{') => self.parse_brace(depth),
            Some(b'[') => self.parse_list(depth),
            Some(b'(') => self.parse_paren(depth),
            Some(b'+' | b'-') => self.parse_signed(depth),
            Some(_other) => self.parse_scalar(depth),
            None => Err(LiteralError::Syntax),
        }
    }

    fn parse_scalar(&mut self, depth: usize) -> Result<Literal, LiteralError> {
        if self.text_start().is_some() {
            return self.parse_text(depth);
        }
        if self.number_start() {
            return self.parse_number();
        }
        if self.take_ellipsis() {
            return Ok(Literal::Ellipsis);
        }
        self.parse_word()
    }

    fn parse_signed(&mut self, depth: usize) -> Result<Literal, LiteralError> {
        self.at = self.at.saturating_add(1);
        if self.skip_trivia(depth)? {
            return Err(LiteralError::Syntax);
        }
        if self.byte() == Some(b'(') {
            let parenthesized = self.parse_paren(depth)?;
            if matches!(parenthesized, Literal::Number) {
                return Ok(parenthesized);
            }
            return Err(LiteralError::NotLiteral);
        }
        if !self.number_start() {
            return Err(LiteralError::NotLiteral);
        }
        self.parse_number()
    }

    fn parse_list(&mut self, depth: usize) -> Result<Literal, LiteralError> {
        self.open(depth)?;
        if self.byte() == Some(b']') {
            self.at = self.at.saturating_add(1);
            return Ok(Literal::List(Vec::new()));
        }
        let mut items = Vec::new();
        loop {
            if self.byte() == Some(b'*') {
                return Err(LiteralError::NotLiteral);
            }
            items.push(self.parse_value(depth.saturating_add(1))?);
            if !self.close_or_comma(depth, b']')? {
                return Ok(Literal::List(items));
            }
        }
    }

    fn parse_paren(&mut self, depth: usize) -> Result<Literal, LiteralError> {
        self.open(depth)?;
        if self.byte() == Some(b')') {
            self.at = self.at.saturating_add(1);
            return Ok(Literal::Tuple(Vec::new()));
        }
        if self.byte() == Some(b'*') {
            return Err(LiteralError::NotLiteral);
        }
        let first = self.parse_value(depth.saturating_add(1))?;
        self.skip_trivia(depth.saturating_add(1))?;
        if self.byte() == Some(b')') {
            self.at = self.at.saturating_add(1);
            return Ok(first);
        }
        if self.byte() != Some(b',') {
            return Err(LiteralError::Syntax);
        }
        let mut items = vec![first];
        loop {
            if !self.close_or_comma(depth, b')')? {
                return Ok(Literal::Tuple(items));
            }
            if self.byte() == Some(b'*') {
                return Err(LiteralError::NotLiteral);
            }
            items.push(self.parse_value(depth.saturating_add(1))?);
        }
    }

    fn parse_brace(&mut self, depth: usize) -> Result<Literal, LiteralError> {
        self.open(depth)?;
        if self.byte() == Some(b'}') {
            self.at = self.at.saturating_add(1);
            return Ok(Literal::Dict(Vec::new()));
        }
        if self.byte() == Some(b'*') {
            return Err(LiteralError::NotLiteral);
        }
        let first = self.parse_value(depth.saturating_add(1))?;
        self.skip_trivia(depth.saturating_add(1))?;
        match self.byte() {
            Some(b':') => self.parse_dict(depth, first),
            Some(b',' | b'}') => self.parse_set_elements(depth, first),
            _other => Err(LiteralError::Syntax),
        }
    }

    fn parse_dict(&mut self, depth: usize, first: Literal) -> Result<Literal, LiteralError> {
        let mut entries = Vec::new();
        let mut key = first;
        loop {
            self.at = self.at.saturating_add(1);
            self.skip_trivia(depth.saturating_add(1))?;
            let value = self.parse_value(depth.saturating_add(1))?;
            replace_entry(&mut entries, key, value);
            if !self.close_or_comma(depth, b'}')? {
                return Ok(Literal::Dict(entries));
            }
            key = self.parse_value(depth.saturating_add(1))?;
            self.skip_trivia(depth.saturating_add(1))?;
            if self.byte() != Some(b':') {
                return Err(LiteralError::Syntax);
            }
        }
    }

    fn parse_set_elements(
        &mut self,
        depth: usize,
        first: Literal,
    ) -> Result<Literal, LiteralError> {
        let mut items = vec![first];
        loop {
            if !self.close_or_comma(depth, b'}')? {
                return Ok(Literal::Set(items));
            }
            if self.byte() == Some(b'*') {
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
        while let Some(byte) = self.byte() {
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
        self.skip_trivia(1)?;
        if self.byte() != Some(b'(') {
            return Err(LiteralError::NotLiteral);
        }
        self.at = self.at.saturating_add(1);
        self.skip_trivia(1)?;
        if self.byte() != Some(b')') {
            return Err(LiteralError::NotLiteral);
        }
        self.at = self.at.saturating_add(1);
        Ok(Literal::Set(Vec::new()))
    }
}

// a repeated text key keeps its first position and its last value, as Python's dict does
fn replace_entry(entries: &mut Vec<(Literal, Literal)>, key: Literal, value: Literal) {
    let Literal::Str(text) = &key else {
        entries.push((key, value));
        return;
    };
    let duplicate = entries.iter_mut().find(|entry| match &entry.0 {
        Literal::Str(existing) => existing == text,
        _other => false,
    });
    match duplicate {
        Some(entry) => entry.1 = value,
        None => entries.push((key, value)),
    }
}
