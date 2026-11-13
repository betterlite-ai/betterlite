use regex_syntax::hir::ClassUnicode;

use super::{Flags, RegexError, Translation, characters, encoded};

mod escapes;
mod groups;

struct Scanner<'pattern> {
    rest: &'pattern str,
    flags: Flags,
    scopes: Vec<Flags>,
    output: String,
    classes: Option<Vec<ClassUnicode>>,
    folded_refs: bool,
}

pub(super) fn translate(pattern: &str, encode: bool) -> Result<Translation, RegexError> {
    let mut scanner = Scanner {
        rest: pattern,
        flags: Flags::default(),
        scopes: Vec::new(),
        output: String::new(),
        classes: encode.then(Vec::new),
        folded_refs: false,
    };
    while let Some(character) = scanner.take() {
        scanner.token(character)?;
        if scanner.output.len() > 0x0010_0000 || scanner.scopes.len() > 128 {
            return Err(RegexError::Limit);
        }
    }
    if !scanner.scopes.is_empty() {
        return Err(RegexError::Syntax);
    }
    Ok(Translation {
        source: scanner.output,
        classes: scanner.classes,
        folded_refs: scanner.folded_refs,
    })
}

impl Scanner<'_> {
    fn take(&mut self) -> Option<char> {
        let mut characters = self.rest.chars();
        let character = characters.next()?;
        self.rest = characters.as_str();
        Some(character)
    }

    fn token(&mut self, character: char) -> Result<(), RegexError> {
        match character {
            '#' if self.flags.verbose => {
                self.rest = self.rest.split_once('\n').map_or("", |(_, tail)| tail);
            }
            ' ' | '\t' | '\n' | '\r' | '\u{b}' | '\u{c}' if self.flags.verbose => {}
            '(' => groups::group(self)?,
            ')' => {
                self.flags = self.scopes.pop().ok_or(RegexError::Syntax)?;
                self.output.push(')');
            }
            '[' => {
                let content = self.class_content()?;
                self.output.push_str(&characters::character_set(
                    &content,
                    self.flags,
                    &mut self.classes,
                )?);
            }
            '\\' => {
                let escaped = escapes::escape(self)?;
                self.output.push_str(&escaped);
            }
            '.' | '^' | '$' if self.classes.is_some() => {
                self.output.push_str(&encoded::operator(
                    character,
                    self.flags.newlines,
                    &mut self.classes,
                )?);
            }
            '.' | '*' | '+' | '?' | '|' | '^' | '$' => self.output.push(character),
            _ => self.output.push_str(&characters::literal(
                character,
                self.flags,
                &mut self.classes,
            )),
        }
        Ok(())
    }

    fn class_content(&mut self) -> Result<String, RegexError> {
        let mut content = String::new();
        if self.rest.starts_with('^') {
            self.take();
            content.push('^');
        }
        loop {
            content.push_str(&self.class_range()?);
            if let Some(tail) = self.rest.strip_prefix(']') {
                self.rest = tail;
                return Ok(content);
            }
        }
    }

    fn class_range(&mut self) -> Result<String, RegexError> {
        let start = self.class_atom()?;
        match self.rest.strip_prefix('-') {
            None => Ok(start),
            Some(tail) if tail.starts_with(']') => {
                self.rest = tail;
                Ok(format!(r"{start}\-"))
            }
            Some(tail) => {
                self.rest = tail;
                Ok(format!("{start}-{}", self.class_atom()?))
            }
        }
    }

    fn class_atom(&mut self) -> Result<String, RegexError> {
        match self.take().ok_or(RegexError::Syntax)? {
            '\\' => self.class_escape(),
            literal @ ('[' | ']' | '-' | '&' | '~' | '|') => Ok(format!(r"\{literal}")),
            literal => Ok(literal.to_string()),
        }
    }

    fn class_escape(&mut self) -> Result<String, RegexError> {
        let escaped = self.take().ok_or(RegexError::Syntax)?;
        if escaped == 'b' {
            return Ok(r"\x08".to_owned());
        }
        let length = match escaped {
            'x' => 2,
            'u' => 4,
            'U' => 8,
            '0'..='7' => self
                .rest
                .chars()
                .take(2)
                .take_while(|digit| ('0'..='7').contains(digit))
                .count(),
            _ => 0,
        };
        let digits = self.rest.get(..length).ok_or(RegexError::Syntax)?;
        let atom = format!(r"\{escaped}{digits}");
        self.rest = self.rest.get(length..).ok_or(RegexError::Syntax)?;
        Ok(atom)
    }
}
