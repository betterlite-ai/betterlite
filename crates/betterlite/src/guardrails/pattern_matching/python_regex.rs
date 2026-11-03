use std::iter::Peekable;
use std::str::Chars;

use fancy_regex::{Error, ParseError, Regex};

const BOUNDARY: &str =
    r"(?-i:(?:(?<=[\p{L}\p{N}_])(?![\p{L}\p{N}_])|(?<![\p{L}\p{N}_])(?=[\p{L}\p{N}_])))";

#[derive(Clone, Copy)]
struct Flags {
    case_insensitive: bool,
    verbose: bool,
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
            '\\' => escaped(&mut self.output, characters.next(), false)?,
            '[' => self.character_class(characters)?,
            '(' => self.group(characters),
            ')' => {
                self.flags = self.scopes.pop().unwrap_or(self.flags);
                self.output.push(character);
            }
            '#' if self.flags.verbose => {
                self.output.push(character);
                copy_until(&mut self.output, characters, '\n');
            }
            '$' => self.output.push_str(r"(?:$|(?=\n\z))"),
            'i' | 'I' | '\u{130}' | '\u{131}' if self.flags.case_insensitive => {
                self.output.push_str("[iI\u{130}\u{131}]");
            }
            other => self.output.push(other),
        }
        Ok(())
    }

    fn character_class(&mut self, characters: &mut Peekable<Chars<'_>>) -> Result<(), Error> {
        let mut class = "[".to_owned();
        if characters.next_if_eq(&'^').is_some() {
            class.push('^');
        }
        if characters.next_if_eq(&']').is_some() {
            class.push(']');
        }
        while let Some(character) = characters.next() {
            if character == ']' {
                return self.finish_class(class);
            }
            if character == '\\' {
                escaped(&mut class, characters.next(), true)?;
                continue;
            }
            class.push(character);
        }
        self.output.push_str(&class);
        Ok(())
    }

    fn finish_class(&mut self, mut class: String) -> Result<(), Error> {
        if self.flags.case_insensitive {
            let positive = class
                .strip_prefix("[^")
                .map_or_else(|| class.clone(), |body| format!("[{body}"));
            let probe = Regex::new(&format!("(?i){positive}]"))?;
            if probe.is_match("i")? || probe.is_match("\u{130}")? || probe.is_match("\u{131}")? {
                class.push_str("iI\u{130}\u{131}");
            }
        }
        class.push(']');
        self.output.push_str(&class);
        Ok(())
    }

    fn group(&mut self, characters: &mut Peekable<Chars<'_>>) {
        self.output.push('(');
        self.scopes.push(self.flags);
        if characters.next_if_eq(&'?').is_none() {
            return;
        }
        self.output.push('?');
        match characters.peek().copied() {
            Some('#') => {
                copy_until(&mut self.output, characters, ')');
                self.scopes.pop();
            }
            Some('i' | 'm' | 's' | 'x' | '-') => self.flag_group(characters),
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
    }

    fn flag_group(&mut self, characters: &mut Peekable<Chars<'_>>) {
        let mut enabled = true;
        for character in characters.by_ref() {
            self.output.push(character);
            match character {
                '-' => enabled = false,
                'i' => self.flags.case_insensitive = enabled,
                'x' => self.flags.verbose = enabled,
                ')' => {
                    self.scopes.pop();
                    break;
                }
                ':' => break,
                _ => {}
            }
        }
    }
}

fn escaped(output: &mut String, character: Option<char>, in_class: bool) -> Result<(), Error> {
    match character {
        Some('A' | 'B' | 'Z') if in_class => {
            return Err(Error::ParseError(output.len(), ParseError::InvalidClass));
        }
        Some('b') if !in_class => output.push_str(BOUNDARY),
        Some('B') => {
            output.push_str(r"(?!\A\z)(?!");
            output.push_str(BOUNDARY);
            output.push(')');
        }
        Some('b') => output.push_str(r"\x08"),
        Some('s') => output.push_str(r"[\s\x{1c}-\x{1f}]"),
        Some('S') => output.push_str(r"[^\s\x{1c}-\x{1f}]"),
        Some('w') => output.push_str(r"[\p{L}\p{N}_]"),
        Some('W') => output.push_str(r"[^\p{L}\p{N}_]"),
        Some('Z') => output.push_str(r"\z"),
        Some(other) => {
            output.push('\\');
            output.push(other);
        }
        None => output.push('\\'),
    }
    Ok(())
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
