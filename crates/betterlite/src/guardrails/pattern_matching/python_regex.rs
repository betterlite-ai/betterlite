use std::iter::{Peekable, from_fn};
use std::str::Chars;

use fancy_regex::{Error, ParseError, Regex, RegexBuilder};

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

#[derive(Default)]
struct Detected {
    ascii_backref: bool,
    unicode: bool,
}

struct Translator {
    output: String,
    flags: Flags,
    scopes: Vec<Flags>,
    detected: Detected,
}

pub(super) fn compile(source: &str) -> Result<Regex, Error> {
    let mut translated = Translator {
        output: "(?i)".to_owned(),
        flags: Flags {
            case_insensitive: true,
            verbose: false,
            alphabet: Alphabet::Unicode,
        },
        scopes: Vec::new(),
        detected: Detected::default(),
    };
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        translated.token(character, &mut characters)?;
    }
    let unicode = !translated.detected.ascii_backref || translated.detected.unicode;
    if !unicode {
        translated.output.insert_str(0, "(?-u)");
    }
    RegexBuilder::new(&translated.output)
        .unicode_mode(unicode)
        .build()
}

pub(super) fn builder(regex: &Regex) -> RegexBuilder {
    let mut builder = RegexBuilder::new(regex.as_str());
    // Translated sources record the engine mode so every rebuild retains it.
    builder.unicode_mode(!regex.as_str().starts_with("(?-u)"));
    builder
}

impl Translator {
    fn token(
        &mut self,
        character: char,
        characters: &mut Peekable<Chars<'_>>,
    ) -> Result<(), Error> {
        match character {
            '\\' => {
                let (escaped, wide, backref) = escapes::translate(characters, self.flags)?;
                self.detected.unicode |= wide;
                self.detected.ascii_backref |= backref;
                self.output.push_str(&escaped);
            }
            '[' => {
                let (translated, wide) = classes::translate(characters, self.flags)?;
                self.detected.unicode |= wide;
                self.output.push_str(&translated);
            }
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
            value if value.is_alphabetic() => {
                self.detected.unicode |=
                    self.flags.alphabet == Alphabet::Unicode && self.flags.case_insensitive;
                self.output.push_str(&self.flags.literal(value));
            }
            other => {
                self.detected.unicode |= other == '.';
                self.output.push(other);
            }
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
        let next = characters.peek().copied();
        if matches!(next, Some('a' | 'u' | 'i' | 'm' | 's' | 'x' | '-')) {
            return self.flag_group(characters);
        }
        if next == Some('#') {
            copy_until(&mut self.output, characters, ')');
            self.scopes.pop();
        } else if next == Some('P') {
            self.named_group(characters);
        } else if next == Some('(') {
            self.output.push_str(&condition_header(characters));
        }
        Ok(())
    }

    fn named_group(&mut self, characters: &mut Peekable<Chars<'_>>) {
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
            self.detected.ascii_backref |= self.flags.alphabet == Alphabet::Ascii;
            self.detected.unicode |=
                self.flags.alphabet == Alphabet::Unicode && self.flags.case_insensitive;
        }
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
        self.output.push_str(&self.flag_header(enabled, disabled));
        let delimiter = characters
            .next()
            .ok_or(Error::ParseError(0, ParseError::UnclosedOpenParen))?;
        self.output.push(delimiter);
        if delimiter == ')' {
            self.scopes.pop();
        }
        Ok(())
    }

    fn flag_header(&self, enabled: &str, disabled: &str) -> String {
        let enabled_out: String = enabled
            .chars()
            .filter(|value| !matches!(value, 'a' | 'u'))
            .collect();
        match (enabled_out.is_empty(), disabled.is_empty()) {
            (true, true) if self.flags.case_insensitive => "i".to_owned(),
            (true, true) => "-i".to_owned(),
            (_, true) => enabled_out,
            (_, false) => format!("{enabled_out}-{disabled}"),
        }
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
