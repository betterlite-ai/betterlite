use super::super::{Alphabet, Flags, RegexError};
use super::Scanner;

pub(super) fn group(scanner: &mut Scanner<'_>) -> Result<(), RegexError> {
    if scanner.rest.starts_with("?#") {
        return skip_comment(scanner);
    }
    scanner.output.push('(');
    if scanner.rest.starts_with("?P=") {
        scanner.folded_refs |= scanner.flags.caseless;
        copy_through(scanner, ')')?;
        return Ok(());
    }
    scanner.scopes.push(scanner.flags);
    if scanner.rest.starts_with("?P<") {
        copy_through(scanner, '>')?;
        return Ok(());
    }
    if scanner.rest.starts_with("?(") {
        copy_through(scanner, ')')?;
        return Ok(());
    }
    if scanner.rest.starts_with('?') {
        scanner.take();
        scanner.output.push('?');
        group_prefix(scanner)?;
    }
    Ok(())
}

fn group_prefix(scanner: &mut Scanner<'_>) -> Result<(), RegexError> {
    if scanner.rest.starts_with([':', '=', '!', '>']) {
        let character = scanner.take().ok_or(RegexError::Syntax)?;
        scanner.output.push(character);
    } else if scanner.rest.starts_with("<=") || scanner.rest.starts_with("<!") {
        let prefix = scanner.take().ok_or(RegexError::Syntax)?;
        let kind = scanner.take().ok_or(RegexError::Syntax)?;
        scanner.output.push(prefix);
        scanner.output.push(kind);
    } else {
        flag_group(scanner)?;
    }
    Ok(())
}

fn copy_through(scanner: &mut Scanner<'_>, end: char) -> Result<(), RegexError> {
    loop {
        let character = scanner.take().ok_or(RegexError::Syntax)?;
        scanner.output.push(character);
        if character == end {
            return Ok(());
        }
    }
}

fn flag_group(scanner: &mut Scanner<'_>) -> Result<(), RegexError> {
    let mut enabled = String::new();
    let mut disabled = String::new();
    let mut negate = false;
    loop {
        let flag = scanner.take().ok_or(RegexError::Syntax)?;
        match flag {
            ':' => break,
            '-' => negate = true,
            'a' if !negate => scanner.flags.alphabet = Alphabet::Ascii,
            'u' if !negate => scanner.flags.alphabet = Alphabet::Unicode,
            'i' | 'm' | 's' | 'x' if negate => {
                scanner.flags.update(flag, false);
                disabled.push(flag);
            }
            'i' | 'm' | 's' | 'x' => {
                scanner.flags.update(flag, true);
                enabled.push(flag);
            }
            _ => return Err(RegexError::Syntax),
        }
    }
    if scanner.classes.is_some() {
        if scanner.flags.alphabet == Alphabet::Ascii {
            enabled.push('r');
        } else {
            disabled.push('r');
        }
    }
    scanner.output.push_str(&enabled);
    if !disabled.is_empty() {
        scanner.output.push('-');
        scanner.output.push_str(&disabled);
    }
    scanner.output.push(':');
    Ok(())
}

impl Flags {
    const fn update(&mut self, flag: char, enabled: bool) {
        match flag {
            'i' => self.caseless = enabled,
            'x' => self.verbose = enabled,
            's' => self.newlines.dot = enabled,
            'm' => self.newlines.anchors = enabled,
            _ => {}
        }
    }
}

fn skip_comment(scanner: &mut Scanner<'_>) -> Result<(), RegexError> {
    loop {
        match scanner.take().ok_or(RegexError::Syntax)? {
            ')' => return Ok(()),
            '\\' => {
                scanner.take().ok_or(RegexError::Syntax)?;
            }
            _ => {}
        }
    }
}
