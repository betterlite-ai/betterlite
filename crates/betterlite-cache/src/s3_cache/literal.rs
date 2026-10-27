use std::collections::BTreeMap;
use std::num::{ParseFloatError, ParseIntError};
use std::str::{Chars, Utf8Error, from_utf8};

use serde_json::{Map, Number, Value};

use super::CachedEntry;

pub(crate) fn to_dictionary(value: &Value) -> Result<CachedEntry, super::super::body::DecodeError> {
    let entries = value
        .as_array()
        .ok_or(super::super::body::DecodeError::Dictionary)?;
    let mut dictionary = BTreeMap::new();
    for entry in entries {
        if let Some(pair) = entry.as_array() {
            let [key, payload] = pair.as_slice() else {
                return Err(super::super::body::DecodeError::Dictionary);
            };
            let name = key
                .as_str()
                .ok_or(super::super::body::DecodeError::Dictionary)?;
            dictionary.insert(name.to_owned(), CachedEntry(payload.clone()));
        } else if let Some(text) = entry.as_str() {
            let mut chars = text.chars();
            let key = chars
                .next()
                .ok_or(super::super::body::DecodeError::Dictionary)?;
            let payload = chars
                .next()
                .ok_or(super::super::body::DecodeError::Dictionary)?;
            if chars.next().is_some() {
                return Err(super::super::body::DecodeError::Dictionary);
            }
            dictionary.insert(
                key.to_string(),
                CachedEntry(Value::String(payload.to_string())),
            );
        } else {
            return Err(super::super::body::DecodeError::Dictionary);
        }
    }
    serde_json::to_value(dictionary)
        .map(CachedEntry)
        .map_err(|source| super::super::body::DecodeError::Json {
            source: Box::new(source),
        })
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Value, LiteralError> {
    let text = from_utf8(bytes).map_err(|source| LiteralError::Utf8 { source })?;
    let mut parser = Parser {
        chars: text.chars(),
    };
    let value = parser.value()?;
    parser.space();
    if parser.chars.next().is_some() {
        return Err(LiteralError::Trailing);
    }
    Ok(value)
}

struct Parser<'input> {
    chars: Chars<'input>,
}

impl Parser<'_> {
    fn value(&mut self) -> Result<Value, LiteralError> {
        self.space();
        match self.chars.next().ok_or(LiteralError::UnexpectedEnd)? {
            '{' => self.mapping(),
            '[' => self.sequence(']'),
            '(' => self.sequence(')'),
            quote @ ('\'' | '"') => self.string(quote),
            character if character.is_ascii_digit() || character == '-' => self.number(character),
            character if character.is_ascii_alphabetic() => self.word(character),
            _ => Err(LiteralError::Unexpected),
        }
    }

    fn mapping(&mut self) -> Result<Value, LiteralError> {
        let mut object = Map::new();
        loop {
            self.space();
            if self.consume('}') {
                return Ok(Value::Object(object));
            }
            let key = self.value()?.as_str().ok_or(LiteralError::Key)?.to_owned();
            self.space();
            self.consume(':')
                .then_some(())
                .ok_or(LiteralError::Unexpected)?;
            object.insert(key, self.value()?);
            self.space();
            if !self.consume(',') {
                self.consume('}')
                    .then_some(())
                    .ok_or(LiteralError::Unexpected)?;
                return Ok(Value::Object(object));
            }
        }
    }

    fn sequence(&mut self, closing: char) -> Result<Value, LiteralError> {
        let mut values = Vec::new();
        loop {
            self.space();
            if self.consume(closing) {
                return Ok(Value::Array(values));
            }
            values.push(self.value()?);
            self.space();
            if !self.consume(',') {
                self.consume(closing)
                    .then_some(())
                    .ok_or(LiteralError::Unexpected)?;
                return Ok(Value::Array(values));
            }
        }
    }

    fn string(&mut self, quote: char) -> Result<Value, LiteralError> {
        let mut value = String::new();
        while let Some(next_character) = self.chars.next() {
            match next_character {
                closing if closing == quote => return Ok(Value::String(value)),
                '\\' => value.push(self.chars.next().ok_or(LiteralError::UnexpectedEnd)?),
                other => value.push(other),
            }
        }
        Err(LiteralError::UnexpectedEnd)
    }

    fn number(&mut self, first: char) -> Result<Value, LiteralError> {
        let mut text = first.to_string();
        while let Some(character) = self.chars.clone().next() {
            if character.is_ascii_digit() || matches!(character, '.' | 'e' | 'E' | '+' | '-') {
                text.push(character);
                self.chars.next();
            } else {
                break;
            }
        }
        if !text.contains(['.', 'e', 'E']) {
            return text
                .parse::<i64>()
                .map(Number::from)
                .map(Value::Number)
                .map_err(|source| LiteralError::Integer {
                    source: Box::new(source),
                });
        }
        let number = text.parse::<f64>().map_err(|source| LiteralError::Float {
            source: Box::new(source),
        })?;
        Number::from_f64(number)
            .map(Value::Number)
            .ok_or(LiteralError::Number)
    }

    fn word(&mut self, first: char) -> Result<Value, LiteralError> {
        let mut word = first.to_string();
        while let Some(character) = self.chars.clone().next() {
            if character.is_ascii_alphabetic() {
                word.push(character);
                self.chars.next();
            } else {
                break;
            }
        }
        match word.as_str() {
            "None" => Ok(Value::Null),
            "True" => Ok(Value::Bool(true)),
            "False" => Ok(Value::Bool(false)),
            _ => Err(LiteralError::Word),
        }
    }

    fn space(&mut self) {
        while self
            .chars
            .clone()
            .next()
            .is_some_and(|character| character.is_ascii_whitespace())
        {
            self.chars.next();
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        if self.chars.clone().next() == Some(expected) {
            self.chars.next();
            true
        } else {
            false
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LiteralError {
    #[error("cache literal is not UTF-8")]
    Utf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("cache literal ended unexpectedly")]
    UnexpectedEnd,
    #[error("cache literal has unexpected syntax")]
    Unexpected,
    #[error("cache literal has trailing data")]
    Trailing,
    #[error("cache literal has a non-string dictionary key")]
    Key,
    #[error("cache literal has an unsupported word")]
    Word,
    #[error("cache literal has an invalid number")]
    Number,
    #[error("cache literal has an invalid integer")]
    Integer {
        #[source]
        source: Box<ParseIntError>,
    },
    #[error("cache literal has an invalid float")]
    Float {
        #[source]
        source: Box<ParseFloatError>,
    },
}
