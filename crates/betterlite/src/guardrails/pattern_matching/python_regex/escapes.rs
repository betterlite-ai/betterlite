use std::iter::Peekable;
use std::str::Chars;

use fancy_regex::{Error, ParseError};

use super::{Alphabet, Flags};

impl Flags {
    pub(super) fn literal(self, value: char) -> String {
        if self.alphabet == Alphabet::Ascii {
            if self.case_insensitive && value.is_ascii_alphabetic() {
                return format!(
                    "(?-i:[{}{}])",
                    value.to_ascii_lowercase(),
                    value.to_ascii_uppercase()
                );
            }
            return format!(r"(?-i:\x{{{:x}}})", u32::from(value));
        }
        if self.case_insensitive && matches!(value, 'i' | 'I' | '\u{130}' | '\u{131}') {
            "[iI\u{130}\u{131}]".to_owned()
        } else {
            format!(r"\x{{{:x}}}", u32::from(value))
        }
    }

    pub(super) fn category(self, character: char) -> String {
        let body = match (
            character.to_ascii_lowercase(),
            self.alphabet == Alphabet::Ascii,
        ) {
            ('w', true) => "a-zA-Z0-9_",
            ('w', false) => r"\p{L}\p{N}_",
            ('s', true) => r" \t\n\r\x0b\x0c",
            ('s', false) => r"\s\x{1c}-\x{1f}",
            ('d', true) => "0-9",
            _ => r"\p{Decimal_Number}",
        };
        let negated = if character.is_uppercase() { "^" } else { "" };
        format!("(?-i:[{negated}{body}])")
    }
}

pub(super) fn translate(
    characters: &mut Peekable<Chars<'_>>,
    flags: Flags,
) -> Result<String, Error> {
    let character = characters
        .next()
        .ok_or(Error::ParseError(0, ParseError::TrailingBackslash))?;
    if let Some(value) = literal(character, characters, false)? {
        return Ok(flags.literal(value));
    }
    Ok(match character {
        'w' | 'W' | 's' | 'S' | 'd' | 'D' => flags.category(character),
        'b' | 'B' => {
            let word = flags.category('w');
            let boundary = format!("(?:(?<={word})(?!{word})|(?<!{word})(?={word}))");
            if character == 'b' {
                boundary
            } else {
                format!(r"(?!\A\z)(?!{boundary})")
            }
        }
        'Z' => r"\z".to_owned(),
        '1'..='9' => {
            let mut reference = format!(r"\{character}");
            if let Some(digit) = characters.next_if(char::is_ascii_digit) {
                reference.push(digit);
            }
            reference
        }
        other => format!(r"\{other}"),
    })
}

pub(super) fn literal(
    character: char,
    characters: &mut Peekable<Chars<'_>>,
    in_class: bool,
) -> Result<Option<char>, Error> {
    let digits = match character {
        'x' => 2_u8,
        'u' => 4,
        'U' => 8,
        '0'..='7' => return octal(character, characters, in_class),
        'N' => return named(characters).map(Some),
        'a' => return Ok(Some('\x07')),
        'f' => return Ok(Some('\x0c')),
        'n' => return Ok(Some('\n')),
        'r' => return Ok(Some('\r')),
        't' => return Ok(Some('\t')),
        'v' => return Ok(Some('\x0b')),
        _ => return Ok(None),
    };
    let mut value = 0_u32;
    for _ in 0..digits {
        let digit = characters
            .next()
            .and_then(|item| item.to_digit(16))
            .ok_or(Error::ParseError(0, ParseError::InvalidHex))?;
        value = value
            .checked_mul(16)
            .and_then(|number| number.checked_add(digit))
            .ok_or(Error::ParseError(0, ParseError::InvalidCodepointValue))?;
    }
    char::from_u32(value)
        .map(Some)
        .ok_or(Error::ParseError(0, ParseError::InvalidCodepointValue))
}

fn octal(
    first: char,
    characters: &mut Peekable<Chars<'_>>,
    in_class: bool,
) -> Result<Option<char>, Error> {
    let suffix: String = characters
        .clone()
        .take(2)
        .take_while(|value| matches!(value, '0'..='7'))
        .collect();
    if !in_class && first != '0' && suffix.len() != 2 {
        return Ok(None);
    }
    let source = format!("{first}{suffix}");
    let value = u32::from_str_radix(&source, 8)
        .ok()
        .filter(|number| *number <= 0xff)
        .and_then(char::from_u32)
        .ok_or(Error::ParseError(0, ParseError::InvalidCodepointValue))?;
    for _ in suffix.chars() {
        characters.next();
    }
    Ok(Some(value))
}

fn named(characters: &mut Peekable<Chars<'_>>) -> Result<char, Error> {
    if characters.next_if_eq(&'{').is_none() {
        return Err(Error::ParseError(0, ParseError::UnclosedUnicodeName));
    }
    while characters.next_if(|value| *value != '}').is_some() {}
    if characters.next_if_eq(&'}').is_none() {
        return Err(Error::ParseError(0, ParseError::UnclosedUnicodeName));
    }
    Err(Error::ParseError(0, ParseError::InvalidCodepointValue))
}
