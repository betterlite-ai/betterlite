use std::iter::{Peekable, from_fn};
use std::str::Chars;

use fancy_regex::{Error, ParseError, Regex};

mod classes;
mod escapes;

#[derive(Clone, Copy, PartialEq)]
enum Alphabet {
    Ascii,
    Unicode,
}

#[derive(Clone, Copy)]
struct Flags {
    case_insensitive: bool,
    verbose: bool,
    alphabet: Alphabet,
}

struct Translator {
    output: String,
    flags: Flags,
    scopes: Vec<Flags>,
}

pub(super) fn compile(source: &str) -> Result<Regex, Error> {
    let mut translator = Translator {
        output: "(?i)".to_owned(),
        flags: Flags {
            case_insensitive: true,
            verbose: false,
            alphabet: Alphabet::Unicode,
        },
        scopes: Vec::new(),
    };
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        translator.token(character, &mut characters)?;
    }
    Regex::new(&translator.output)
}

impl Translator {
    fn token(
        &mut self,
        character: char,
        characters: &mut Peekable<Chars<'_>>,
    ) -> Result<(), Error> {
        match character {
            '\\' => {
                let escaped = escapes::translate(characters, self.flags)?;
                self.output.push_str(&escaped);
            }
            '[' => self
                .output
                .push_str(&classes::translate(characters, self.flags)?),
            '(' => self.group(characters)?,
            ')' => {
                self.flags = self.scopes.pop().unwrap_or(self.flags);
                self.output.push(character);
            }
            '#' if self.flags.verbose => {
                self.output.push(character);
                copy_until(&mut self.output, characters, '\n');
            }
            '$' => self.output.push_str(r"(?:$|(?=\n\z))"),
            value if value.is_alphabetic() => self.output.push_str(&self.flags.literal(value)),
            other => self.output.push(other),
        }
        Ok(())
    }

    fn group(&mut self, characters: &mut Peekable<Chars<'_>>) -> Result<(), Error> {
        self.output.push('(');
        self.scopes.push(self.flags);
        if characters.next_if_eq(&'?').is_none() {
            return Ok(());
        }
        self.output.push('?');
        match characters.peek().copied() {
            Some('#') => {
                copy_until(&mut self.output, characters, ')');
                self.scopes.pop();
            }
            Some('a' | 'u' | 'i' | 'm' | 's' | 'x' | '-') => self.flag_group(characters)?,
            Some('P') => {
                self.output.push('P');
                characters.next();
                let end = if characters.peek() == Some(&'=') {
                    ')'
                } else {
                    '>'
                };
                copy_until(&mut self.output, characters, end);
                if end == ')' {
                    self.scopes.pop();
                }
            }
            Some('(') => self.output.push_str(&condition_header(characters)),
            _ => {}
        }
        Ok(())
    }

    fn flag_group(&mut self, characters: &mut Peekable<Chars<'_>>) -> Result<(), Error> {
        let header: String =
            from_fn(|| characters.next_if(|value| !matches!(value, ':' | ')'))).collect();
        let (enabled, disabled) = header.split_once('-').unwrap_or((&header, ""));
        if (enabled.contains('a') && enabled.contains('u')) || disabled.contains(['a', 'u']) {
            return Err(Error::ParseError(0, ParseError::UnknownFlag(header)));
        }
        self.flags.case_insensitive =
            (self.flags.case_insensitive || enabled.contains('i')) && !disabled.contains('i');
        self.flags.verbose =
            (self.flags.verbose || enabled.contains('x')) && !disabled.contains('x');
        if enabled.contains('a') {
            self.flags.alphabet = Alphabet::Ascii;
        }
        if enabled.contains('u') {
            self.flags.alphabet = Alphabet::Unicode;
        }
        self.output.push_str(&header.replace('a', "u"));
        let delimiter = characters
            .next()
            .ok_or(Error::ParseError(0, ParseError::UnclosedOpenParen))?;
        self.output.push(delimiter);
        if delimiter == ')' {
            self.scopes.pop();
        }
        Ok(())
    }
}

fn copy_until(output: &mut String, characters: &mut Peekable<Chars<'_>>, end: char) {
    for character in characters.by_ref() {
        output.push(character);
        if character == end {
            break;
        }
    }
}

fn condition_header(characters: &mut Peekable<Chars<'_>>) -> String {
    characters.next();
    let name: String = characters
        .by_ref()
        .take_while(|value| *value != ')')
        .collect();
    if name.starts_with(|value: char| value.is_alphabetic() || value == '_') {
        format!("(<{name}>)")
    } else {
        format!("({name})")
    }
}
