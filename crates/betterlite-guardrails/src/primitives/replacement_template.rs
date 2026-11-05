use std::iter::Peekable;
use std::mem;
use std::str::Chars;

use fancy_regex::{Captures, Regex};

#[derive(Debug, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    Group(usize),
}

pub fn parse(template: &str, regex: &Regex) -> Option<Vec<Piece>> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut chars = template.chars().peekable();
    while let Some(current) = chars.next() {
        if current != '\\' {
            literal.push(current);
            continue;
        }
        let escaped = chars.next()?;
        match escaped {
            'n' => literal.push('\n'),
            't' => literal.push('\t'),
            'r' => literal.push('\r'),
            'a' => literal.push('\x07'),
            'b' => literal.push('\x08'),
            'f' => literal.push('\x0c'),
            'v' => literal.push('\x0b'),
            '\\' => literal.push('\\'),
            '0' => literal.push('\0'),
            'g' => {
                let group = named_reference(&mut chars, regex)?;
                pieces.push(Piece::Text(mem::take(&mut literal)));
                pieces.push(Piece::Group(group));
            }
            '1'..='9' => {
                let mut number = escaped.to_string();
                number.extend(chars.next_if(char::is_ascii_digit));
                let group = resolve(&number, regex)?;
                pieces.push(Piece::Text(mem::take(&mut literal)));
                pieces.push(Piece::Group(group));
            }
            other if other.is_ascii_alphabetic() => return None,
            other => {
                literal.push('\\');
                literal.push(other);
            }
        }
    }
    pieces.push(Piece::Text(literal));
    Some(pieces)
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
