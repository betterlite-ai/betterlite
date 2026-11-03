use super::{Flags, RegexError, characters};

mod escapes;
mod groups;

struct Scanner<'pattern> {
    rest: &'pattern str,
    flags: Flags,
    scopes: Vec<Flags>,
    output: String,
}

pub(super) fn translate(pattern: &str) -> Result<String, RegexError> {
    let mut scanner = Scanner {
        rest: pattern,
        flags: Flags::default(),
        scopes: Vec::new(),
        output: String::new(),
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
    Ok(scanner.output)
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
                self.output
                    .push_str(&characters::character_set(&content, self.flags)?);
            }
            '\\' => {
                let escaped = escapes::escape(self)?;
                self.output.push_str(&escaped);
            }
            '.' | '*' | '+' | '?' | '|' | '^' | '$' => self.output.push(character),
            _ => self
                .output
                .push_str(&characters::literal(character, self.flags)),
        }
        Ok(())
    }

    fn class_content(&mut self) -> Result<String, RegexError> {
        let mut content = String::new();
        if self.rest.starts_with('^') {
            self.take();
            content.push('^');
        }
        if self.rest.starts_with(']') {
            self.take();
            content.push_str(r"\]");
        }
        loop {
            match self.take().ok_or(RegexError::Syntax)? {
                ']' => return Ok(content),
                '\\' => {
                    let escaped = self.take().ok_or(RegexError::Syntax)?;
                    content.push_str(&self.class_escape(escaped));
                }
                literal @ ('[' | '&' | '~') => {
                    content.push('\\');
                    content.push(literal);
                }
                literal => content.push(literal),
            }
        }
    }
    fn class_escape(&self, escaped: char) -> String {
        characters::perl_class(escaped, self.flags).unwrap_or_else(|| {
            if escaped == 'b' {
                r"\x08".to_owned()
            } else {
                format!(r"\{escaped}")
            }
        })
    }
}
