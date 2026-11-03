use std::iter::Peekable;
use std::str::Chars;

use fancy_regex::{Error, ParseError, Regex};

use super::{Alphabet, Flags, escapes};

pub(super) fn translate(
    characters: &mut Peekable<Chars<'_>>,
    flags: Flags,
) -> Result<String, Error> {
    let negated = characters.next_if_eq(&'^').is_some();
    let mut literals = String::new();
    let mut alternatives = Vec::new();
    if characters.next_if_eq(&']').is_some() {
        literals.push_str(r"\]");
    }
    while let Some(character) = characters.next() {
        if character == ']' {
            return finish(literals, alternatives, flags, negated);
        }
        match character {
            '\\' => escaped(characters, &mut literals, &mut alternatives, flags)?,
            '[' | '^' | '&' | '~' => {
                literals.push('\\');
                literals.push(character);
            }
            '-' if characters.peek() == Some(&']') => literals.push_str(r"\-"),
            other => literals.push(other),
        }
    }
    Err(Error::ParseError(0, ParseError::InvalidClass))
}

fn finish(
    literals: String,
    mut alternatives: Vec<String>,
    flags: Flags,
    negated: bool,
) -> Result<String, Error> {
    if !literals.is_empty() {
        alternatives.push(fold(literals, flags)?);
    }
    let choices = alternatives.join("|");
    Ok(if negated {
        format!("(?!(?:{choices}))(?s:.)")
    } else {
        format!("(?:{choices})")
    })
}

fn escaped(
    characters: &mut Peekable<Chars<'_>>,
    literals: &mut String,
    alternatives: &mut Vec<String>,
    flags: Flags,
) -> Result<(), Error> {
    let character = characters
        .next()
        .ok_or(Error::ParseError(0, ParseError::InvalidClass))?;
    match character {
        'w' | 'W' | 's' | 'S' | 'd' | 'D' => alternatives.push(flags.category(character)),
        'b' => literals.push_str(r"\x08"),
        'A' | 'B' | 'Z' => return Err(Error::ParseError(0, ParseError::InvalidClass)),
        other => {
            if let Some(value) = escapes::literal(other, characters, true)? {
                let encoded = format!(r"\x{{{:x}}}", u32::from(value));
                literals.push_str(&encoded);
            } else {
                literals.push('\\');
                literals.push(other);
            }
        }
    }
    Ok(())
}

fn fold(mut literals: String, flags: Flags) -> Result<String, Error> {
    let prefix = if flags.alphabet == Alphabet::Ascii || !flags.case_insensitive {
        ""
    } else {
        "(?i)"
    };
    let probe = Regex::new(&format!("{prefix}[{literals}]"))?;
    let candidates = if flags.alphabet == Alphabet::Ascii {
        "abcdefghijklmnopqrstuvwxyz"
    } else {
        "i\u{130}\u{131}"
    };
    for value in candidates.chars().filter(|_| flags.case_insensitive) {
        if !probe.is_match(&value.to_string())?
            && !probe.is_match(&value.to_ascii_uppercase().to_string())?
        {
            continue;
        }
        if flags.alphabet == Alphabet::Ascii {
            literals.push(value);
            literals.push(value.to_ascii_uppercase());
        } else {
            literals.push_str("iI\u{130}\u{131}");
            break;
        }
    }
    if flags.alphabet == Alphabet::Ascii {
        Ok(format!("(?-i:[{literals}])"))
    } else {
        Ok(format!("[{literals}]"))
    }
}
