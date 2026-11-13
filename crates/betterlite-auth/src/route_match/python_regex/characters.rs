use std::fmt::Write as _;

use regex_syntax::ParserBuilder;
use regex_syntax::ast::{self, Ast, ClassPerl, ClassPerlKind, ClassSet, ClassSetItem};
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

pub(super) fn literal(
    character: char,
    flags: Flags,
    registry: &mut Option<Vec<ClassUnicode>>,
) -> String {
    let mut class = ClassUnicode::new([ClassUnicodeRange::new(character, character)]);
    if flags.caseless {
        fold(&mut class, flags);
    }
    render(&class, registry)
}

pub(super) fn character_set(
    source: &str,
    flags: Flags,
    registry: &mut Option<Vec<ClassUnicode>>,
) -> Result<String, RegexError> {
    let ast = ast::parse::ParserBuilder::new()
        .octal(true)
        .build()
        .parse(&format!("[{source}]"))
        .map_err(|error| RegexError::CharacterSet(Box::new(regex_syntax::Error::Parse(error))))?;
    let Ast::ClassBracketed(bracketed) = &ast else {
        return Err(RegexError::Syntax);
    };
    let ClassSet::Item(item) = &bracketed.kind else {
        return Err(RegexError::Syntax);
    };
    let mut class = ClassUnicode::empty();
    let mut pending = vec![item];
    while let Some(member) = pending.pop() {
        if let ClassSetItem::Union(union) = member {
            pending.extend(&union.items);
        } else {
            class.union(&class_member(member, flags)?);
        }
    }
    if bracketed.negated {
        class.negate();
    }
    Ok(render(&class, registry))
}

fn class_member(member: &ClassSetItem, flags: Flags) -> Result<ClassUnicode, RegexError> {
    let range = match member {
        ClassSetItem::Literal(literal) => ClassUnicodeRange::new(literal.c, literal.c),
        ClassSetItem::Range(range) => ClassUnicodeRange::new(range.start.c, range.end.c),
        // Folding an expanded category would add nonmembers such as U+0345 to \w.
        ClassSetItem::Perl(perl) => return category(perl, flags),
        ClassSetItem::Empty(_)
        | ClassSetItem::Ascii(_)
        | ClassSetItem::Unicode(_)
        | ClassSetItem::Bracketed(_)
        | ClassSetItem::Union(_) => return Err(RegexError::Syntax),
    };
    let mut class = ClassUnicode::new([range]);
    if flags.caseless {
        fold(&mut class, flags);
    }
    Ok(class)
}

fn category(perl: &ClassPerl, flags: Flags) -> Result<ClassUnicode, RegexError> {
    let kind = match perl.kind {
        ClassPerlKind::Digit => 'd',
        ClassPerlKind::Space => 's',
        ClassPerlKind::Word => 'w',
    };
    let letter = if perl.negated {
        kind.to_ascii_uppercase()
    } else {
        kind
    };
    let source = perl_class(letter, flags).ok_or(RegexError::Syntax)?;
    let hir = ParserBuilder::new()
        .build()
        .parse(&source)
        .map_err(|error| RegexError::CharacterSet(Box::new(error)))?;
    let HirKind::Class(Class::Unicode(class)) = hir.kind() else {
        return Err(RegexError::Syntax);
    };
    Ok(class.clone())
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

fn render(class: &ClassUnicode, registry: &mut Option<Vec<ClassUnicode>>) -> String {
    if let Some(classes) = registry {
        let index = classes
            .iter()
            .position(|known| known == class)
            .unwrap_or_else(|| {
                let next = classes.len();
                classes.push(class.clone());
                next
            });
        return format!("(?-i:\0{index}\0)");
    }
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
