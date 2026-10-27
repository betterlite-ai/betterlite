use std::iter::Peekable;
use std::num::{ParseFloatError, ParseIntError};
use std::str::{Chars, Utf8Error, from_utf8};

use serde_json::{Map, Number, Value};

pub(crate) fn parse(bytes: &[u8]) -> Result<Value, LiteralError> {
    let text = from_utf8(bytes).map_err(LiteralError::Utf8)?;
    let mut parser = Parser {
        chars: text.chars().peekable(),
        depth: 0,
    };
    let value = parser.value()?;
    parser.space();
    if parser.chars.next().is_some() {
        return Err(LiteralError::Unexpected);
    }
    Ok(value)
}

struct Parser<'input> {
    chars: Peekable<Chars<'input>>,
    depth: usize,
}

impl Parser<'_> {
    fn value(&mut self) -> Result<Value, LiteralError> {
        self.space();
        match self.chars.next().ok_or(LiteralError::Unexpected)? {
            '{' => self.collection('}'),
            '[' => self.collection(']'),
            '(' => self.collection(')'),
            quote @ ('\'' | '"') => self.string(quote),
            character if character.is_ascii_alphanumeric() || character == '-' => {
                self.token_value(character)
            }
            _ => Err(LiteralError::Unexpected),
        }
    }

    fn collection(&mut self, closing: char) -> Result<Value, LiteralError> {
        if self.depth >= 128 {
            return Err(LiteralError::DepthLimit);
        }
        self.depth = self.depth.saturating_add(1);
        let mut object = Map::new();
        let mut values = Vec::new();
        self.space();
        while self.chars.next_if_eq(&closing).is_none() {
            let item = self.value()?;
            if closing == '}' {
                let key = item.as_str().ok_or(LiteralError::Key)?.to_owned();
                self.space();
                self.chars
                    .next_if_eq(&':')
                    .ok_or(LiteralError::Unexpected)?;
                object.insert(key, self.value()?);
            } else {
                values.push(item);
            }
            self.space();
            if self.chars.next_if_eq(&',').is_none() {
                self.chars
                    .next_if_eq(&closing)
                    .ok_or(LiteralError::Unexpected)?;
                break;
            }
            self.space();
        }
        self.depth = self.depth.saturating_sub(1);
        Ok(if closing == '}' {
            Value::Object(object)
        } else {
            Value::Array(values)
        })
    }

    fn string(&mut self, quote: char) -> Result<Value, LiteralError> {
        let mut value = String::new();
        while let Some(character) = self.chars.next() {
            match character {
                closing if closing == quote => return Ok(Value::String(value)),
                '\\' => self.escape(&mut value)?,
                '\n' | '\r' | '\0' => return Err(LiteralError::Unexpected),
                other => value.push(other),
            }
        }
        Err(LiteralError::Unexpected)
    }

    fn escape(&mut self, value: &mut String) -> Result<(), LiteralError> {
        let escaped = self.chars.next().ok_or(LiteralError::Unexpected)?;
        let decoded = match escaped {
            'a' => '\u{07}',
            'b' => '\u{08}',
            'f' => '\u{0c}',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'v' => '\u{0b}',
            '\\' | '\'' | '"' => escaped,
            'x' => self.hex_escape(2)?,
            'u' => self.hex_escape(4)?,
            'U' => self.hex_escape(8)?,
            'N' => self.named_escape()?,
            '0'..='7' => self.octal_escape(escaped)?,
            '\n' => return Ok(()),
            '\r' => {
                self.chars.next_if_eq(&'\n');
                return Ok(());
            }
            other => {
                value.push('\\');
                other
            }
        };
        value.push(decoded);
        Ok(())
    }

    fn hex_escape(&mut self, length: usize) -> Result<char, LiteralError> {
        let digits: String = self.chars.by_ref().take(length).collect();
        if digits.len() != length || !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
            return Err(LiteralError::Escape);
        }
        let code = u32::from_str_radix(&digits, 16).map_err(LiteralError::Integer)?;
        char::from_u32(code).ok_or(LiteralError::Escape)
    }

    fn octal_escape(&mut self, first: char) -> Result<char, LiteralError> {
        let mut digits = first.to_string();
        for _ in 0_u8..2 {
            if let Some(digit @ '0'..='7') = self.chars.peek().copied() {
                digits.push(digit);
                self.chars.next();
            } else {
                break;
            }
        }
        let code = u32::from_str_radix(&digits, 8).map_err(LiteralError::Integer)?;
        char::from_u32(code).ok_or(LiteralError::Escape)
    }

    fn named_escape(&mut self) -> Result<char, LiteralError> {
        if self.chars.next_if_eq(&'{').is_none() {
            return Err(LiteralError::Escape);
        }
        let mut name = String::new();
        for character in self.chars.by_ref() {
            if character == '}' {
                return unicode_names2::character(&name).ok_or(LiteralError::Escape);
            }
            if name.len() >= 128 {
                return Err(LiteralError::Escape);
            }
            name.push(character);
        }
        Err(LiteralError::Unexpected)
    }

    fn token(&mut self, first: char, allowed: fn(char) -> bool) -> String {
        let mut token = first.to_string();
        while let Some(character) = self.chars.next_if(|character| allowed(*character)) {
            token.push(character);
        }
        token
    }

    fn token_value(&mut self, first: char) -> Result<Value, LiteralError> {
        if first.is_ascii_alphabetic() {
            return match self
                .token(first, |value| value.is_ascii_alphabetic())
                .as_str()
            {
                "None" => Ok(Value::Null),
                "True" => Ok(Value::Bool(true)),
                "False" => Ok(Value::Bool(false)),
                _ => Err(LiteralError::Unexpected),
            };
        }
        let text = self.token(first, |value| {
            value.is_ascii_digit() || matches!(value, '.' | 'e' | 'E' | '+' | '-')
        });
        if !text.contains(['.', 'e', 'E']) {
            return text
                .parse::<i64>()
                .map(Number::from)
                .map(Value::Number)
                .map_err(LiteralError::Integer);
        }
        let number = text.parse::<f64>().map_err(LiteralError::Float)?;
        Number::from_f64(number)
            .map(Value::Number)
            .ok_or(LiteralError::Number)
    }

    fn space(&mut self) {
        while self.chars.next_if(char::is_ascii_whitespace).is_some() {}
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LiteralError {
    #[error("cache literal is not UTF-8")]
    Utf8(#[source] Utf8Error),
    #[error("cache literal has unexpected syntax")]
    Unexpected,
    #[error("cache literal has a non-string dictionary key")]
    Key,
    #[error("cache literal has an invalid number")]
    Number,
    #[error("cache literal nesting exceeds 128 levels")]
    DepthLimit,
    #[error("cache literal has an invalid string escape")]
    Escape,
    #[error("cache literal has an invalid integer")]
    Integer(#[source] ParseIntError),
    #[error("cache literal has an invalid float")]
    Float(#[source] ParseFloatError),
}
