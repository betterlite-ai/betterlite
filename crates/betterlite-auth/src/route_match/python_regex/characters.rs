use std::fmt::Write as _;
use std::str;

use regex_syntax::ParserBuilder;
use regex_syntax::hir::{Class, ClassUnicode, ClassUnicodeRange, HirKind};

use super::{Alphabet, Flags, RegexError};

pub(super) fn perl_class(character: char, flags: Flags) -> Option<String> {
    let content = match (
        character.to_ascii_lowercase(),
        (flags.alphabet == Alphabet::Ascii),
    ) {
        ('w', true) => "a-zA-Z0-9_",
        ('w', false) => r"\p{L}\p{N}_",
        ('d', true) => "0-9",
        ('d', false) => r"\d",
        ('s', true) => r"\t\n\x0b\x0c\r ",
        ('s', false) => r"\t\n\x0b\x0c\r\x1c-\x1f\x85\p{Z}",
        _ => return None,
    };
    let negate = if character.is_ascii_uppercase() {
        "^"
    } else {
        ""
    };
    Some(format!("[{negate}{content}]"))
}

pub(super) fn literal(character: char, flags: Flags) -> String {
    if !flags.caseless {
        return format!(r"\x{{{:x}}}", u32::from(character));
    }
    let mut class = ClassUnicode::new([ClassUnicodeRange::new(character, character)]);
    fold(&mut class, flags);
    render(&class)
}

pub(super) fn character_set(source: &str, flags: Flags) -> Result<String, RegexError> {
    let (negated, content) = source
        .strip_prefix('^')
        .map_or((false, source), |tail| (true, tail));
    let hir = ParserBuilder::new()
        .octal(true)
        .build()
        .parse(&format!("[{content}]"))
        .map_err(|error| RegexError::CharacterSet(Box::new(error)))?;
    let mut class = match hir.kind() {
        HirKind::Class(Class::Unicode(class)) => class.clone(),
        HirKind::Literal(literal) => {
            let text = str::from_utf8(&literal.0).map_err(RegexError::Utf8)?;
            ClassUnicode::new(
                text.chars()
                    .map(|value| ClassUnicodeRange::new(value, value)),
            )
        }
        HirKind::Empty
        | HirKind::Class(Class::Bytes(_))
        | HirKind::Look(_)
        | HirKind::Repetition(_)
        | HirKind::Capture(_)
        | HirKind::Concat(_)
        | HirKind::Alternation(_) => return Err(RegexError::Syntax),
    };
    if flags.caseless {
        fold(&mut class, flags);
    }
    if negated {
        class.negate();
    }
    Ok(render(&class))
}

fn fold(class: &mut ClassUnicode, flags: Flags) {
    if flags.alphabet == Alphabet::Ascii {
        let mut ascii = class.clone();
        ascii.intersect(&ClassUnicode::new([ClassUnicodeRange::new('\0', '\u{7f}')]));
        ascii.case_fold_simple();
        ascii.intersect(&ClassUnicode::new([ClassUnicodeRange::new('\0', '\u{7f}')]));
        class.union(&ascii);
    } else {
        // Python adds both Turkish i variants to the Unicode simple folding equivalence class.
        let turkish = ['I', 'i', '\u{130}', '\u{131}'];
        if class.ranges().iter().any(|range| {
            turkish
                .iter()
                .any(|value| range.start() <= *value && *value <= range.end())
        }) {
            class.union(&ClassUnicode::new(
                turkish.map(|value| ClassUnicodeRange::new(value, value)),
            ));
        }
        class.case_fold_simple();
    }
}

fn render(class: &ClassUnicode) -> String {
    if class.ranges().is_empty() {
        return "(?!)".to_owned();
    }
    let mut output = String::from("(?-i:[");
    for range in class.ranges() {
        let _formatted = write!(
            output,
            r"\x{{{:x}}}-\x{{{:x}}}",
            u32::from(range.start()),
            u32::from(range.end())
        );
    }
    output.push_str("])");
    output
}
