use std::fmt::Write as _;

use super::super::PatternProblem;
use super::Walker;

impl Walker<'_> {
    pub(super) fn group(&mut self) -> Result<(), PatternProblem> {
        let mut probe = self.input.clone();
        let Some('?') = probe.next() else {
            return self.open_group();
        };
        match probe.next() {
            Some('#') => self.comment_group(),
            Some('<') => Err(PatternProblem::Syntax),
            Some('P') => self.named_group(),
            Some('a' | 'i' | 'L' | 'm' | 's' | 'u' | 'U' | 'R' | 'x' | '-') => self.flags_group(),
            _ => self.open_group(),
        }
    }

    fn open_group(&mut self) -> Result<(), PatternProblem> {
        self.output.push('(');
        let verbose = matches!(self.verbose.last(), Some(true));
        self.verbose.push(verbose);
        self.at_start = false;
        Ok(())
    }

    fn named_group(&mut self) -> Result<(), PatternProblem> {
        let mut probe = self.input.clone();
        probe.next();
        probe.next();
        if probe.next() != Some('<') {
            return self.open_group();
        }
        let mut name = String::new();
        loop {
            match probe.next() {
                Some('>') => break,
                Some(character) => name.push(character),
                None => return Err(PatternProblem::Syntax),
            }
        }
        if !identifier(&name) {
            return Err(PatternProblem::Syntax);
        }
        self.output.push_str("(?P<");
        self.output.push_str(&name);
        self.output.push('>');
        for _ in 0..name.chars().count().saturating_add(4) {
            let _ = self.input.next();
        }
        let verbose = matches!(self.verbose.last(), Some(true));
        self.verbose.push(verbose);
        self.at_start = false;
        Ok(())
    }

    fn flags_group(&mut self) -> Result<(), PatternProblem> {
        let mut probe = self.input.clone();
        probe.next();
        let mut spec = String::new();
        let terminator = loop {
            match probe.next() {
                Some(character @ (':' | ')')) => break character,
                Some(letter @ ('a' | 'i' | 'L' | 'm' | 's' | 'u' | 'U' | 'R' | 'x' | '-')) => {
                    spec.push(letter);
                }
                _ => return self.open_group(),
            }
        };
        let (added, removed) = kept_flags(&spec)?;
        for _ in 0..spec.chars().count().saturating_add(2) {
            let _ = self.input.next();
        }
        let inherited = matches!(self.verbose.last(), Some(true));
        let verbose = if added.contains('x') {
            true
        } else if removed.contains('x') {
            false
        } else {
            inherited
        };
        if terminator == ')' {
            // a global flag group can only turn flags on, and only at the start of the expression
            if !self.at_start || spec.contains('-') {
                return Err(PatternProblem::Syntax);
            }
            if !added.is_empty() {
                let _ignored = write!(self.output, "(?{added})");
            }
            if let Some(base) = self.verbose.first_mut() {
                *base = verbose;
            }
            return Ok(());
        }
        let scoped = if removed.is_empty() {
            added
        } else {
            format!("{added}-{removed}")
        };
        let _ignored = write!(self.output, "(?{scoped}:");
        self.verbose.push(verbose);
        self.at_start = false;
        Ok(())
    }

    fn comment_group(&mut self) -> Result<(), PatternProblem> {
        self.input.next();
        self.input.next();
        loop {
            match self.input.next() {
                Some('\\') => {
                    if self.input.next().is_none() {
                        return Err(PatternProblem::Syntax);
                    }
                }
                Some(')') => return Ok(()),
                Some(_) => {}
                None => return Err(PatternProblem::Syntax),
            }
        }
    }
}

fn identifier(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first == '_' || unicode_ident::is_xid_start(first)) {
        return false;
    }
    characters.all(|character| character == '_' || unicode_ident::is_xid_continue(character))
}

// the flags the matcher engine carries, deduped
fn kept_flags(spec: &str) -> Result<(String, String), PatternProblem> {
    let (added, removed) = spec.split_once('-').unwrap_or((spec, ""));
    if removed.contains('-') || (spec.contains('-') && removed.is_empty()) {
        return Err(PatternProblem::Syntax);
    }
    if added.contains('a') && added.contains('u') {
        return Err(PatternProblem::Syntax);
    }
    if added.contains('a') {
        return Err(PatternProblem::Unsupported);
    }
    let mut kept = String::new();
    for letter in added.chars() {
        if !matches!(letter, 'i' | 'm' | 's' | 'u' | 'x') {
            return Err(PatternProblem::Syntax);
        }
        if !kept.contains(letter) {
            kept.push(letter);
        }
    }
    let mut cleared = String::new();
    for letter in removed.chars() {
        if !matches!(letter, 'i' | 'm' | 's' | 'x') || added.contains(letter) {
            return Err(PatternProblem::Syntax);
        }
        if !cleared.contains(letter) {
            cleared.push(letter);
        }
    }
    Ok((kept, cleared))
}
