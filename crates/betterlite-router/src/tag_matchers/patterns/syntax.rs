use std::iter::Peekable;
use std::str::Chars;

// Python and the engine read these constructs differently, so the pattern is skipped instead of reinterpreted
pub(super) fn unsupported_syntax(source: &str) -> bool {
    let mut scanner = Scanner {
        characters: source.chars().peekable(),
        escaped: false,
        class: ClassState::Outside,
    };
    while let Some(character) = scanner.characters.next() {
        if scanner.step(character) {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClassState {
    Outside,
    Open,
    Negated,
    Body,
}

struct Scanner<'source> {
    characters: Peekable<Chars<'source>>,
    escaped: bool,
    class: ClassState,
}

impl Scanner<'_> {
    fn step(&mut self, character: char) -> bool {
        if self.escaped {
            self.escaped = false;
            return self.escape(character);
        }
        if character == '\\' {
            self.escaped = true;
            return false;
        }
        if self.class != ClassState::Outside {
            return self.class_char(character);
        }
        if character == '[' {
            self.class = ClassState::Open;
            return false;
        }
        character == '(' && self.characters.peek() == Some(&'?') && self.group()
    }

    fn escape(&mut self, character: char) -> bool {
        let next = self.characters.peek().copied();
        if (character == 'N' && next == Some('{')) || character == 'Z' {
            return true;
        }
        if self.class != ClassState::Outside {
            self.class = ClassState::Body;
            return false;
        }
        if character == '<' || character == '>' {
            return true;
        }
        if character == '0' {
            return true;
        }
        is_three_digit_octal(character, &self.characters)
    }

    fn group(&mut self) -> bool {
        self.characters.next();
        self.characters.peek() == Some(&'(')
            || self.characters.peek() == Some(&'#')
            || enables_verbose(&self.characters)
    }

    fn class_char(&mut self, character: char) -> bool {
        // only a caret directly after [ keeps the start position, so a second one is a literal
        if self.class == ClassState::Open && character == '^' {
            self.class = ClassState::Negated;
            return false;
        }
        if self.class != ClassState::Body && character == ']' {
            // Python reads ]-x after the opening bracket as a range; []-] is a literal bracket and a hyphen
            if self.characters.clone().next() == Some('-')
                && self.characters.clone().nth(1) != Some(']')
            {
                return true;
            }
            self.class = ClassState::Body;
            return false;
        }
        match class_step(character, self.characters.peek()) {
            ClassStep::Unsupported => true,
            ClassStep::Close => {
                self.class = ClassState::Outside;
                false
            }
            ClassStep::Literal => {
                self.class = ClassState::Body;
                false
            }
        }
    }
}

fn is_three_digit_octal<I>(character: char, characters: &I) -> bool
where
    I: Iterator<Item = char> + Clone,
{
    if !matches!(character, '1'..='3') {
        return false;
    }
    let mut probe = characters.clone();
    let second = probe.next();
    let third = probe.next();
    second.is_some_and(|digit| ('0'..='7').contains(&digit))
        && third.is_some_and(|digit| ('0'..='7').contains(&digit))
}

fn enables_verbose<I>(characters: &I) -> bool
where
    I: Iterator<Item = char> + Clone,
{
    let mut verbose = false;
    let mut disabled = false;
    for character in characters.clone() {
        match character {
            '-' => disabled = true,
            ':' | ')' => return verbose,
            'x' => {
                if !disabled {
                    verbose = true;
                }
            }
            'a' | 'i' | 'L' | 'm' | 's' | 'u' => {}
            _ => return false,
        }
    }
    false
}

enum ClassStep {
    Unsupported,
    Close,
    Literal,
}

fn class_step(character: char, next: Option<&char>) -> ClassStep {
    if character == '[' {
        return ClassStep::Unsupported;
    }
    if character == ']' {
        return ClassStep::Close;
    }
    let doubled = next.is_some_and(|following| {
        (*following == '&' && character == '&')
            || (*following == '~' && character == '~')
            || (*following == '-' && character == '-')
    });
    if doubled {
        return ClassStep::Unsupported;
    }
    ClassStep::Literal
}
