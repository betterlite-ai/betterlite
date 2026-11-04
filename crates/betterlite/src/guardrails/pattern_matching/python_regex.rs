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
    wide_class: bool,
}

struct Translator {
    output: String,
    flags: Flags,
    scopes: Vec<Flags>,
    detected: Detected,
    target_unicode: bool,
}

pub(super) fn compile(source: &str) -> Result<Regex, Error> {
    let translated = translate(source, true)?;
    if translated.detected.ascii_backref && !translated.detected.wide_class {
        let ascii = translate(source, false)?;
        return RegexBuilder::new(&ascii.output).unicode_mode(false).build();
    }
    Regex::new(&translated.output)
}

struct Translation {
    output: String,
    detected: Detected,
}

// Run twice when an ascii backreference is found: detect, then retranslate for the chosen mode.
fn translate(source: &str, target_unicode: bool) -> Result<Translation, Error> {
    let mut translator = Translator {
        output: "(?i)".to_owned(),
        flags: Flags {
            case_insensitive: true,
            verbose: false,
            alphabet: Alphabet::Unicode,
        },
        scopes: Vec::new(),
        detected: Detected::default(),
        target_unicode,
    };
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        translator.token(character, &mut characters)?;
    }
    Ok(Translation {
        output: translator.output,
        detected: translator.detected,
    })
}

impl Translator {
    fn token(
        &mut self,
        character: char,
        characters: &mut Peekable<Chars<'_>>,
    ) -> Result<(), Error> {
        match character {
            '\\' => {
                self.detected.ascii_backref |= self.flags.alphabet == Alphabet::Ascii
                    && matches!(characters.peek(), Some('1'..='9'));
                let (escaped, wide) = escapes::translate(characters, self.flags)?;
                self.detected.wide_class |= wide;
                self.output.push_str(&escaped);
            }
            '[' => {
                let (translated, wide) = classes::translate(characters, self.flags)?;
                self.detected.wide_class |= wide;
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
            '.' => {
                self.detected.wide_class = true;
                self.output.push('.');
            }
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
                    self.detected.ascii_backref |= self.flags.alphabet == Alphabet::Ascii;
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
        self.output
            .push_str(&self.unicode_header(enabled, disabled));
        let delimiter = characters
            .next()
            .ok_or(Error::ParseError(0, ParseError::UnclosedOpenParen))?;
        self.output.push(delimiter);
        if delimiter == ')' {
            self.scopes.pop();
        }
        Ok(())
    }

    // fancy-regex rejects an inline flag that flips its one engine-wide unicode setting.
    fn unicode_header(&self, enabled: &str, disabled: &str) -> String {
        let alphabet = enabled.contains('a') || enabled.contains('u');
        let mut enabled_out: String = enabled
            .chars()
            .filter(|value| !matches!(value, 'a' | 'u'))
            .collect();
        let mut disabled_out = disabled.to_owned();
        if alphabet {
            if self.target_unicode {
                enabled_out.push('u');
            } else {
                disabled_out.push('u');
            }
        }
        if disabled_out.is_empty() {
            enabled_out
        } else {
            format!("{enabled_out}-{disabled_out}")
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
