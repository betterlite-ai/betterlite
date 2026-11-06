use std::iter::Peekable;
use std::mem;
use std::str::Chars;

use fancy_regex::{Captures, Regex};

#[derive(Debug, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    Group(usize),
}

#[must_use]
pub fn parse(template: &str, regex: &Regex) -> Option<Vec<Piece>> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut chars = template.chars().peekable();
    while let Some(current) = chars.next() {
        if current == '\\' {
            let escaped = chars.next()?;
            translate_escape(escaped, &mut chars, regex, &mut literal, &mut pieces)?;
        } else {
            literal.push(current);
        }
    }
    pieces.push(Piece::Text(literal));
    Some(pieces)
}

enum DigitEscape {
    Literal(char),
    Group(String),
}

// Python reads three octal digits as one byte and otherwise one or two digits as a group reference.
fn digit_escape(chars: &mut Peekable<Chars<'_>>, first: char) -> Option<DigitEscape> {
    let second = chars.next_if(char::is_ascii_digit);
    let third = second
        .filter(|digit| ('0'..='7').contains(&first) && ('0'..='7').contains(digit))
        .and_then(|_| chars.next_if(|character| ('0'..='7').contains(character)));
    if let (Some(second_digit), Some(third_digit)) = (second, third) {
        let digits: String = [first, second_digit, third_digit].into_iter().collect();
        let value = u32::from_str_radix(&digits, 8).ok()?;
        if value > 0o377 {
            return None;
        }
        return Some(DigitEscape::Literal(char::from(u8::try_from(value).ok()?)));
    }
    let mut digits = String::from(first);
    digits.extend(second);
    Some(DigitEscape::Group(digits))
}

// Python reads the digits after \0 as an octal byte, masking the value to eight bits.
fn octal_digits(chars: &mut Peekable<Chars<'_>>, limit: usize) -> String {
    let mut digits = String::new();
    while digits.len() < limit {
        let Some(digit) = chars.next_if(|character| ('0'..='7').contains(character)) else {
            break;
        };
        digits.push(digit);
    }
    digits
}

fn translate_escape(
    escaped: char,
    chars: &mut Peekable<Chars<'_>>,
    regex: &Regex,
    literal: &mut String,
    pieces: &mut Vec<Piece>,
) -> Option<()> {
    match escaped {
        'n' => literal.push('\n'),
        't' => literal.push('\t'),
        'r' => literal.push('\r'),
        'a' => literal.push('\x07'),
        'b' => literal.push('\x08'),
        'f' => literal.push('\x0c'),
        'v' => literal.push('\x0b'),
        '\\' => literal.push('\\'),
        '0' => {
            let mut digits = String::from("0");
            digits.push_str(&octal_digits(chars, 2));
            let value = u32::from_str_radix(&digits, 8).ok()?;
            literal.push(char::from(u8::try_from(value & 0xff).ok()?));
        }
        'g' => {
            let group = named_reference(chars, regex)?;
            pieces.push(Piece::Text(mem::take(literal)));
            pieces.push(Piece::Group(group));
        }
        '1'..='9' => match digit_escape(chars, escaped)? {
            DigitEscape::Literal(character) => literal.push(character),
            DigitEscape::Group(number) => {
                let group = resolve(&number, regex)?;
                pieces.push(Piece::Text(mem::take(literal)));
                pieces.push(Piece::Group(group));
            }
        },
        other if other.is_ascii_alphabetic() => return None,
        other => {
            literal.push('\\');
            literal.push(other);
        }
    }
    Some(())
}

#[must_use]
pub fn expand(pieces: &[Piece], captures: &Captures<'_, str>) -> String {
    let mut expanded = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(text) => expanded.push_str(text),
            Piece::Group(index) => {
                expanded.push_str(captures.get(*index).map_or("", |matched| matched.as_str()));
            }
        }
    }
    expanded
}

fn named_reference(chars: &mut Peekable<Chars<'_>>, regex: &Regex) -> Option<usize> {
    chars.next_if_eq(&'<')?;
    let mut reference = String::new();
    loop {
        let next = chars.next()?;
        if next == '>' {
            break;
        }
        reference.push(next);
    }
    resolve(&reference, regex)
}

fn resolve(reference: &str, regex: &Regex) -> Option<usize> {
    if let Ok(index) = reference.parse::<usize>() {
        return (index < regex.captures_len()).then_some(index);
    }
    regex
        .capture_names()
        .position(|name| name == Some(reference))
}
