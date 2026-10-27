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

macro_rules! parse_collection {
    ($parser:expr, $closing:expr, $collection:expr) => {{
        let mut object = Map::new();
        let mut values = Vec::new();
        let mut elements = 0_usize;
        loop {
            $parser.space();
            if $parser.consume($closing) {
                break match $collection {
                    Collection::Mapping => Ok(Value::Object(object)),
                    Collection::Sequence => Ok(Value::Array(values)),
                };
            }
            if matches!($collection, Collection::Mapping) {
                let key = $parser
                    .value()?
                    .as_str()
                    .ok_or(LiteralError::Key)?
                    .to_owned();
                $parser.space();
                $parser
                    .consume(':')
                    .then_some(())
                    .ok_or(LiteralError::Unexpected)?;
                object.insert(key, $parser.value()?);
            } else {
                values.push($parser.value()?);
            }
            elements = elements
                .checked_add(1)
                .ok_or(LiteralError::CollectionLimit)?;
            if elements > 100_000 {
                break Err(LiteralError::CollectionLimit);
            }
            $parser.space();
            if !$parser.consume(',') {
                $parser
                    .consume($closing)
                    .then_some(())
                    .ok_or(LiteralError::Unexpected)?;
                break match $collection {
                    Collection::Mapping => Ok(Value::Object(object)),
                    Collection::Sequence => Ok(Value::Array(values)),
                };
            }
        }
    }};
}

macro_rules! parse_string {
    ($parser:expr, $quote:expr) => {{
        let mut value = String::new();
        while let Some(next_character) = $parser.chars.next() {
            match next_character {
                closing if closing == $quote => return Ok(Value::String(value)),
                '\\' => match $parser.chars.next().ok_or(LiteralError::UnexpectedEnd)? {
                    'n' => value.push('\n'),
                    'r' => value.push('\r'),
                    't' => value.push('\t'),
                    'b' => value.push('\u{08}'),
                    'f' => value.push('\u{0c}'),
                    'v' => value.push('\u{0b}'),
                    escaped => value.push(escaped),
                },
                other => value.push(other),
            }
        }
        Err(LiteralError::UnexpectedEnd)
    }};
}

impl Parser<'_> {
    fn value(&mut self) -> Result<Value, LiteralError> {
        self.space();
        match self.chars.next().ok_or(LiteralError::UnexpectedEnd)? {
            '{' => parse_collection!(self, '}', Collection::Mapping),
            '[' => parse_collection!(self, ']', Collection::Sequence),
            '(' => parse_collection!(self, ')', Collection::Sequence),
            quote @ ('\'' | '"') => parse_string!(self, quote),
            character
                if character.is_ascii_digit()
                    || character == '-'
                    || character.is_ascii_alphabetic() =>
            {
                self.token_value(character)
            }
            _ => Err(LiteralError::Unexpected),
        }
    }

    fn token(&mut self, first: char, allowed: fn(char) -> bool) -> String {
        let mut token = first.to_string();
        while let Some(character) = self.chars.clone().next() {
            if allowed(character) {
                token.push(character);
                self.chars.next();
            } else {
                break;
            }
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
                _ => Err(LiteralError::Word),
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

#[derive(Clone, Copy)]
enum Collection {
    Mapping,
    Sequence,
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
    #[error("cache literal collection is too large")]
    CollectionLimit,
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
