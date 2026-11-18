use std::fmt::Write as _;
use std::iter::Peekable;
use std::str::Chars;

use super::PatternProblem;

mod groups;

pub(super) fn pattern(input: &str) -> Result<String, PatternProblem> {
    let mut walker = Walker {
        input: input.chars().peekable(),
        output: String::with_capacity(input.len()),
        class: false,
        leading: false,
        caret: false,
        at_start: true,
        verbose: vec![false],
    };
    loop {
        let Some(character) = walker.input.next() else {
            return Ok(walker.output);
        };
        if walker.class {
            walker.in_class(character)?;
        } else {
            walker.outside(character)?;
        }
    }
}

struct Walker<'a> {
    input: Peekable<Chars<'a>>,
    output: String,
    class: bool,
    leading: bool,
    caret: bool,
    at_start: bool,
    verbose: Vec<bool>,
}

impl Walker<'_> {
    fn outside(&mut self, character: char) -> Result<(), PatternProblem> {
        match character {
            '\\' => self.escape(false),
            '[' => {
                self.output.push('[');
                self.class = true;
                self.leading = true;
                self.caret = false;
                self.at_start = false;
                Ok(())
            }
            '(' => self.group(),
            ')' => {
                self.output.push(')');
                if self.verbose.len() != 1 {
                    self.verbose.pop();
                }
                self.at_start = false;
                Ok(())
            }
            '{' => self.brace(),
            '#' if matches!(self.verbose.last(), Some(true)) => {
                for character in self.input.by_ref() {
                    if character == '\n' {
                        break;
                    }
                }
                Ok(())
            }
            _ => {
                let verbose = matches!(self.verbose.last(), Some(true));
                let space = matches!(character, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r');
                if verbose && character.is_whitespace() && !space {
                    self.output.extend(character.escape_unicode());
                } else {
                    self.output.push(character);
                }
                if !(verbose && space) {
                    self.at_start = false;
                }
                Ok(())
            }
        }
    }

    fn escape(&mut self, class: bool) -> Result<(), PatternProblem> {
        let Some(character) = self.input.next() else {
            return Err(PatternProblem::Syntax);
        };
        if !class {
            self.at_start = false;
        }
        match character {
            'Z' => self.output.push_str("\\z"),
            'b' if class => self.output.push_str("\\x08"),
            // Python reads up to three octal digits; the matcher engine reads a codepoint escape
            '0'..='7' => {
                let mut value = u32::from(character) - u32::from('0');
                for _ in 0..2 {
                    let Some(digit) = self.input.peek().and_then(|next| next.to_digit(8)) else {
                        break;
                    };
                    value = value.saturating_mul(8).saturating_add(digit);
                    let _ = self.input.next();
                }
                let _ignored = write!(self.output, "\\x{{{value:x}}}");
            }
            'z' | 'p' | 'P' => return Err(PatternProblem::Syntax),
            'x' | 'u' | 'U' if self.input.peek() == Some(&'{') => {
                return Err(PatternProblem::Syntax);
            }
            '<' => self.output.push_str("\\x3c"),
            '>' => self.output.push_str("\\x3e"),
            character if !character.is_ascii() => self.output.extend(character.escape_unicode()),
            _ => {
                self.output.push('\\');
                self.output.push(character);
            }
        }
        Ok(())
    }

    fn in_class(&mut self, character: char) -> Result<(), PatternProblem> {
        if character == '\\' {
            self.leading = false;
            return self.escape(true);
        }
        if self.leading {
            match character {
                ']' => {
                    self.leading = false;
                    self.output.push_str("\\]");
                    return Ok(());
                }
                '^' if !self.caret => {
                    self.caret = true;
                    self.output.push('^');
                    return Ok(());
                }
                '-' => {
                    self.leading = false;
                    self.output.push_str("\\-");
                    return Ok(());
                }
                _ => {}
            }
        } else if character == ']' {
            self.class = false;
            self.output.push(']');
            return Ok(());
        }
        self.leading = false;
        self.class_character(character);
        Ok(())
    }

    fn class_character(&mut self, character: char) {
        let doubled = matches!(
            (character, self.input.peek().copied()),
            ('&', Some('&')) | ('~', Some('~'))
        );
        if doubled || matches!(character, '[' | '#') || character.is_whitespace() {
            self.output.extend(character.escape_unicode());
        } else {
            self.output.push(character);
        }
    }

    fn brace(&mut self) -> Result<(), PatternProblem> {
        self.at_start = false;
        let mut probe = self.input.clone();
        let mut content = String::new();
        let closed = loop {
            match probe.next() {
                Some(character @ ('0'..='9' | ',')) => content.push(character),
                Some('}') => break true,
                _ => break false,
            }
        };
        if !closed || !repetition_content(&content) {
            self.output.push_str("\\{");
            return Ok(());
        }
        for _ in 0..=content.chars().count() {
            let _ = self.input.next();
        }
        self.output.push('{');
        if content.starts_with(',') {
            self.output.push('0');
        }
        self.output.push_str(&content);
        self.output.push('}');
        Ok(())
    }
}

fn repetition_content(content: &str) -> bool {
    let (before, after) = content.split_once(',').unwrap_or((content, ""));
    before.chars().all(|character| character.is_ascii_digit())
        && after.chars().all(|character| character.is_ascii_digit())
        && (!before.is_empty() || content.contains(','))
}
