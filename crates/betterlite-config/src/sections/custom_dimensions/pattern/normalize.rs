use std::fmt::Write as _;
use std::str::Chars;

use super::PatternProblem;

mod groups;

pub(super) fn pattern(input: &str, python_dollar: bool) -> Result<String, PatternProblem> {
    let mut walker = Walker {
        input: input.chars(),
        output: String::with_capacity(input.len()),
        class: ClassState::Outside,
        at_start: true,
        verbose: vec![false],
        multiline: vec![false],
        python_dollar,
    };
    loop {
        let Some(character) = walker.input.next() else {
            return Ok(walker.output);
        };
        if walker.class == ClassState::Outside {
            walker.outside(character)?;
        } else {
            walker.in_class(character)?;
        }
    }
}

struct Walker<'input> {
    input: Chars<'input>,
    output: String,
    class: ClassState,
    at_start: bool,
    verbose: Vec<bool>,
    multiline: Vec<bool>,
    python_dollar: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClassState {
    Outside,
    Leading,
    Negated,
    Body,
}

impl Walker<'_> {
    fn outside(&mut self, character: char) -> Result<(), PatternProblem> {
        match character {
            '\\' => self.escape(false),
            '[' => {
                self.output.push('[');
                self.class = ClassState::Leading;
                self.at_start = false;
                Ok(())
            }
            '(' => groups::group(self),
            ')' => {
                self.output.push(')');
                if self.verbose.len() != 1 {
                    self.verbose.pop();
                    self.multiline.pop();
                }
                self.at_start = false;
                Ok(())
            }
            '$' if self.python_dollar && !matches!(self.multiline.last(), Some(true)) => {
                self.output.push_str("(?:\\n?\\z)");
                self.at_start = false;
                Ok(())
            }
            '{' => {
                self.brace();
                Ok(())
            }
            '#' if matches!(self.verbose.last(), Some(true)) => {
                let _newline = self.input.by_ref().find(|next| *next == '\n');
                Ok(())
            }
            _ => {
                self.literal(character);
                Ok(())
            }
        }
    }

    fn literal(&mut self, character: char) {
        let verbose = matches!(self.verbose.last(), Some(true));
        let space = matches!(character, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r');
        if verbose && space {
            return;
        }
        if verbose && character.is_whitespace() {
            self.output.extend(character.escape_unicode());
        } else {
            self.output.push(character);
        }
        self.at_start = false;
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
            '0'..='7' => self.octal(character),
            'z' | 'p' | 'P' => return Err(PatternProblem::Syntax),
            'x' | 'u' | 'U' if self.input.clone().next() == Some('{') => {
                return Err(PatternProblem::Syntax);
            }
            '<' => self.output.push_str("\\x3c"),
            '>' => self.output.push_str("\\x3e"),
            _ if !character.is_ascii() => self.output.extend(character.escape_unicode()),
            _ => {
                self.output.push('\\');
                self.output.push(character);
            }
        }
        Ok(())
    }

    fn octal(&mut self, character: char) {
        let mut value = u32::from(character).saturating_sub(u32::from('0'));
        let digits = self
            .input
            .clone()
            .take(2)
            .map_while(|next| next.to_digit(8));
        for digit in digits {
            value = value.saturating_mul(8).saturating_add(digit);
            let _digit = self.input.next();
        }
        let _written = write!(self.output, "\\x{{{value:x}}}");
    }

    fn in_class(&mut self, character: char) -> Result<(), PatternProblem> {
        if character == '\\' {
            self.class = ClassState::Body;
            return self.escape(true);
        }
        if self.class == ClassState::Leading && character == '^' {
            self.class = ClassState::Negated;
            self.output.push('^');
            return Ok(());
        }
        let leading = matches!(self.class, ClassState::Leading | ClassState::Negated);
        if leading && matches!(character, ']' | '-') {
            self.class = ClassState::Body;
            self.output.push('\\');
            self.output.push(character);
            return Ok(());
        }
        if character == ']' {
            self.class = ClassState::Outside;
            self.output.push(']');
            return Ok(());
        }
        self.class = ClassState::Body;
        self.class_character(character);
        Ok(())
    }

    fn class_character(&mut self, character: char) {
        let doubled = matches!(
            (character, self.input.clone().next()),
            ('&', Some('&')) | ('~', Some('~'))
        );
        if doubled || matches!(character, '[' | '#') || character.is_whitespace() {
            self.output.extend(character.escape_unicode());
        } else {
            self.output.push(character);
        }
    }

    fn brace(&mut self) {
        self.at_start = false;
        let mut probe = self.input.clone();
        let mut content = String::new();
        let closed = loop {
            match probe.next() {
                Some(character @ '0'..='9') => content.push(character),
                Some(',') if !content.contains(',') => content.push(','),
                Some('}') => break true,
                _ => break false,
            }
        };
        if !closed || content.is_empty() {
            self.output.push_str("\\{");
            return;
        }
        let _closing_brace = self.input.nth(content.chars().count());
        self.output.push('{');
        if content.starts_with(',') {
            self.output.push('0');
        }
        self.output.push_str(&content);
        self.output.push('}');
    }
}
