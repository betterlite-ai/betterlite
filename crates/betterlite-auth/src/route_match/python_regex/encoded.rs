use std::fmt::Write as _;

use regex_syntax::hir::ClassUnicode;

use super::{Flags, Newlines, RegexError, characters};

pub(super) fn pattern(source: &str, count: usize) -> Result<String, RegexError> {
    let mut pieces = source.split('\0');
    let mut output = pieces.next().unwrap_or_default().to_owned();
    while let Some(identifier) = pieces.next() {
        let index = identifier.parse::<usize>().map_err(RegexError::Number)?;
        let tail = count
            .checked_sub(index)
            .and_then(|remaining| remaining.checked_sub(1))
            .ok_or(RegexError::Syntax)?;
        let _formatted = write!(
            output,
            "[Kk\u{212a}]{{{index}}}[k\u{212a}][Kk\u{212a}]{{{tail}}}[0-9a-f]{{6}}[Kk\u{212a}]{{21}}"
        );
        output.push_str(pieces.next().ok_or(RegexError::Syntax)?);
        if output.len() > 0x0010_0000 {
            return Err(RegexError::Limit);
        }
    }
    Ok(output)
}

pub(super) fn text(source: &str, classes: &[ClassUnicode]) -> Result<String, RegexError> {
    let mut output = String::new();
    for character in source.chars() {
        // K/k/kelvin fold together; PCRE2's r flag keeps kelvin distinct in ASCII scopes.
        let member = if character.is_ascii() {
            'k'
        } else {
            '\u{212a}'
        };
        // Membership bits preserve every atom while the native engine still controls backtracking.
        for class in classes {
            let index = class
                .ranges()
                .partition_point(|range| range.end() < character);
            let contains = class
                .ranges()
                .get(index)
                .is_some_and(|range| range.start() <= character);
            output.push(if contains { member } else { 'K' });
        }
        // Python references compare lowercase codepoints, unlike its broader literal folding sets.
        let lower = character.to_lowercase().next().unwrap_or(character);
        let _formatted = write!(output, "{:06x}", u32::from(lower));
        identity(character, &mut output)?;
        if output.len() > 0x0010_0000 {
            return Err(RegexError::Limit);
        }
    }
    Ok(output)
}

fn identity(character: char, output: &mut String) -> Result<(), RegexError> {
    // Identity bits keep sensitive references exact, with only the ASCII letter case bit using k.
    for shift in (0..21).rev() {
        let bit = 1_u32.checked_shl(shift).ok_or(RegexError::Limit)?;
        let symbol = if u32::from(character) & bit == 0 {
            'K'
        } else if character.is_ascii_alphabetic() && shift == 5 {
            'k'
        } else {
            '\u{212a}'
        };
        output.push(symbol);
    }
    Ok(())
}

pub(super) fn operator(
    character: char,
    newlines: Newlines,
    classes: &mut Option<Vec<ClassUnicode>>,
) -> Result<String, RegexError> {
    if character == '.' {
        let source = if newlines.dot { r"[\s\S]" } else { r"[^\n]" };
        return characters::character_set(source, Flags::default(), classes);
    }
    let newline = characters::literal('\n', Flags::default(), classes);
    Ok(match (character, newlines.anchors) {
        ('^', false) => r"\A".to_owned(),
        ('^', true) => format!(r"(?:\A|(?<={newline}))"),
        ('$', false) => format!(r"(?=(?:{newline})?\z)"),
        ('$', true) => format!(r"(?={newline}|\z)"),
        _ => return Err(RegexError::Syntax),
    })
}
