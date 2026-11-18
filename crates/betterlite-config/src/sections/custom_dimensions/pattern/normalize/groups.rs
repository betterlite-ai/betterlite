use std::fmt::Write as _;

use super::super::PatternProblem;
use super::Walker;

const FLAG_LETTERS: [char; 10] = ['a', 'i', 'L', 'm', 's', 'u', 'U', 'R', 'x', '-'];

pub(super) fn group(walker: &mut Walker<'_>) -> Result<(), PatternProblem> {
    let mut probe = walker.input.clone();
    match (probe.next(), probe.next()) {
        (Some('?'), Some('#')) => {
            let _prefix = walker.input.nth(1);
            loop {
                match walker.input.next() {
                    Some('\\') => {
                        walker.input.next().ok_or(PatternProblem::Syntax)?;
                    }
                    Some(')') => return Ok(()),
                    Some(_) => {}
                    None => return Err(PatternProblem::Syntax),
                }
            }
        }
        (Some('?'), Some('<')) => Err(PatternProblem::Syntax),
        (Some('?'), Some('P')) => named_group(walker),
        (Some('?'), Some('a' | 'i' | 'L' | 'm' | 's' | 'u' | 'U' | 'R' | 'x' | '-')) => {
            flags_group(walker)
        }
        _ => {
            walker.output.push('(');
            let verbose = matches!(walker.verbose.last(), Some(true));
            walker.verbose.push(verbose);
            let multiline = matches!(walker.multiline.last(), Some(true));
            walker.multiline.push(multiline);
            walker.at_start = false;
            Ok(())
        }
    }
}

fn named_group(walker: &mut Walker<'_>) -> Result<(), PatternProblem> {
    let mut probe = walker.input.clone();
    if probe.nth(2) != Some('<') {
        return Err(PatternProblem::Syntax);
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
    let _written = write!(walker.output, "(?P<{name}>");
    let _closing_angle = walker.input.nth(name.chars().count().saturating_add(3));
    let verbose = matches!(walker.verbose.last(), Some(true));
    walker.verbose.push(verbose);
    let multiline = matches!(walker.multiline.last(), Some(true));
    walker.multiline.push(multiline);
    walker.at_start = false;
    Ok(())
}

fn flags_group(walker: &mut Walker<'_>) -> Result<(), PatternProblem> {
    let mut probe = walker.input.clone().skip(1);
    let mut spec = String::new();
    loop {
        let Some(letter) = probe.clone().next() else {
            return Err(PatternProblem::Syntax);
        };
        if !FLAG_LETTERS.contains(&letter) {
            break;
        }
        spec.push(letter);
        let _letter = probe.next();
    }
    let candidate = probe.next().filter(|letter| matches!(letter, ':' | ')'));
    let terminator = candidate.ok_or(PatternProblem::Syntax)?;
    apply_flags(walker, &spec, terminator)
}

fn apply_flags(
    walker: &mut Walker<'_>,
    spec: &str,
    terminator: char,
) -> Result<(), PatternProblem> {
    let (added, removed) = kept_flags(spec)?;
    let _terminator = walker.input.nth(spec.chars().count().saturating_add(1));
    let verbose = added.contains('x')
        || (!removed.contains('x') && matches!(walker.verbose.last(), Some(true)));
    if terminator == ')' {
        if !walker.at_start || spec.contains('-') {
            return Err(PatternProblem::Syntax);
        }
        if !added.is_empty() {
            let _ignored = write!(walker.output, "(?{added})");
        }
        if let Some(base) = walker.verbose.first_mut() {
            *base = verbose;
        }
        if let Some(base) = walker.multiline.first_mut() {
            *base = added.contains('m') || (!removed.contains('m') && *base);
        }
        return Ok(());
    }
    let scoped = if removed.is_empty() {
        added
    } else {
        format!("{added}-{removed}")
    };
    let _ignored = write!(walker.output, "(?{scoped}:");
    walker.verbose.push(verbose);
    let multiline = added.contains('m')
        || (!removed.contains('m') && matches!(walker.multiline.last(), Some(true)));
    walker.multiline.push(multiline);
    walker.at_start = false;
    Ok(())
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
