use super::super::{RegexError, characters};
use super::Scanner;

pub(super) fn escape(scanner: &mut Scanner<'_>) -> Result<String, RegexError> {
    let escaped = scanner.take().ok_or(RegexError::Syntax)?;
    if let Some(class) = characters::perl_class(escaped, scanner.flags) {
        return if scanner.classes.is_some() {
            characters::character_set(&class, scanner.flags, &mut scanner.classes)
        } else {
            Ok(format!("(?-i:{class})"))
        };
    }
    let literal = match escaped {
        'b' | 'B' => return boundary(scanner, escaped),
        'Z' => return Ok(r"\z".to_owned()),
        'A' | 'z' => return Ok(format!(r"\{escaped}")),
        'x' => return hex_literal(scanner, 2),
        'u' => return hex_literal(scanner, 4),
        'U' => return hex_literal(scanner, 8),
        '0'..='9' => return number(scanner, escaped),
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        'v' => '\u{b}',
        'f' => '\u{c}',
        'a' => '\u{7}',
        unknown if unknown.is_ascii_alphabetic() => return Err(RegexError::Syntax),
        character => character,
    };
    Ok(characters::literal(
        literal,
        scanner.flags,
        &mut scanner.classes,
    ))
}

fn hex_literal(scanner: &mut Scanner<'_>, count: usize) -> Result<String, RegexError> {
    let digits = scanner.rest.get(..count).ok_or(RegexError::Syntax)?;
    let codepoint = u32::from_str_radix(digits, 16).map_err(RegexError::Number)?;
    scanner.rest = scanner.rest.get(count..).ok_or(RegexError::Syntax)?;
    let character = char::from_u32(codepoint).ok_or(RegexError::Syntax)?;
    Ok(characters::literal(
        character,
        scanner.flags,
        &mut scanner.classes,
    ))
}

fn number(scanner: &mut Scanner<'_>, first: char) -> Result<String, RegexError> {
    let mut digits = first.to_string();
    let following = scanner.rest.chars().take(2).collect::<String>();
    let octal = first == '0'
        || (('0'..='7').contains(&first)
            && following.len() == 2
            && following.chars().all(|digit| ('0'..='7').contains(&digit)));
    let limit = if octal { 3 } else { 2 };
    while digits.len() < limit
        && scanner.rest.starts_with(|character: char| {
            if octal {
                ('0'..='7').contains(&character)
            } else {
                character.is_ascii_digit()
            }
        })
    {
        digits.push(scanner.take().ok_or(RegexError::Syntax)?);
    }
    if octal {
        let codepoint = u32::from_str_radix(&digits, 8).map_err(RegexError::Number)?;
        let character = char::from_u32(codepoint).ok_or(RegexError::Syntax)?;
        Ok(characters::literal(
            character,
            scanner.flags,
            &mut scanner.classes,
        ))
    } else {
        scanner.folded_refs |= scanner.flags.caseless;
        Ok(format!(r"\g{{{digits}}}"))
    }
}

fn boundary(scanner: &mut Scanner<'_>, kind: char) -> Result<String, RegexError> {
    let class = characters::perl_class('w', scanner.flags).ok_or(RegexError::Syntax)?;
    let word = if scanner.classes.is_some() {
        characters::character_set(&class, scanner.flags, &mut scanner.classes)?
    } else {
        class
    };
    let expression = if kind == 'b' {
        format!("(?:(?<!{word})(?={word})|(?<={word})(?!{word}))")
    } else {
        // The pinned Python version does not match an entirely empty subject with \B.
        format!(r"(?:(?=[\s\S])|(?<=[\s\S]))(?:(?<!{word})(?!{word})|(?<={word})(?={word}))")
    };
    Ok(format!("(?-i:{expression})"))
}
