use std::collections::BTreeSet;

// Starlette's compile_path: literal text matches in place, each placeholder takes what its convertor's regex takes, and the route must consume the template whole
#[must_use]
pub(crate) fn template_matches(template: &str, route: &str) -> bool {
    match parse(template) {
        Some(pieces) => matches_pieces(&pieces, route),
        None => false,
    }
}

// one stretch of literal text and the placeholder that follows it, if any
struct Piece<'template> {
    literal: &'template str,
    convertor: Option<Convertor>,
}

#[derive(Clone, Copy)]
enum Convertor {
    Text,
    Path,
    Int,
    Float,
    Uuid,
}

impl Convertor {
    fn accepts(self, candidate: &str) -> bool {
        match self {
            Self::Text => !candidate.is_empty() && !candidate.contains('/'),
            Self::Path => match candidate.strip_suffix('\n') {
                Some(without) => !without.contains('\n'),
                None => !candidate.contains('\n'),
            },
            Self::Int => {
                !candidate.is_empty() && candidate.bytes().all(|byte| byte.is_ascii_digit())
            }
            Self::Float => float(candidate),
            Self::Uuid => uuid(candidate),
        }
    }
}

// [0-9]+(\.[0-9]+)?
fn float(candidate: &str) -> bool {
    match candidate.split_once('.') {
        Some((whole, fraction)) => {
            Convertor::Int.accepts(whole) && Convertor::Int.accepts(fraction)
        }
        None => Convertor::Int.accepts(candidate),
    }
}

// [0-9a-fA-F]{8}-?[0-9a-fA-F]{4}-?[0-9a-fA-F]{4}-?[0-9a-fA-F]{4}-?[0-9a-fA-F]{12}
fn uuid(candidate: &str) -> bool {
    let mut rest = candidate;
    for (position, length) in [8_usize, 4, 4, 4, 12].into_iter().enumerate() {
        let group_start = if position == 0 {
            rest
        } else {
            rest.strip_prefix('-').unwrap_or(rest)
        };
        let Some((group, tail)) = group_start.split_at_checked(length) else {
            return false;
        };
        if !group.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return false;
        }
        rest = tail;
    }
    rest.is_empty()
}

// Starlette's PARAM_REGEX: a name starts with a letter or underscore, a brace group that is not a placeholder stays literal text, and an unknown convertor fails the path
fn parse(template: &str) -> Option<Vec<Piece<'_>>> {
    let mut pieces = Vec::new();
    let mut literal_start = 0;
    for (index, character) in template.char_indices() {
        if character != '{' {
            continue;
        }
        let Some(rest) = template
            .get(index..)
            .and_then(|text| text.strip_prefix('{'))
        else {
            continue;
        };
        match group(rest) {
            Group::Placeholder { width, convertor } => {
                pieces.push(Piece {
                    literal: template.get(literal_start..index)?,
                    convertor: Some(convertor),
                });
                literal_start = index.saturating_add(width);
            }
            Group::Literal => {}
            Group::Unknown => return None,
        }
    }
    pieces.push(Piece {
        literal: template.get(literal_start..)?,
        convertor: None,
    });
    Some(pieces)
}

// what the brace group right after a "{" amounts to
enum Group {
    Placeholder { width: usize, convertor: Convertor },
    Literal,
    Unknown,
}

fn group(rest: &str) -> Group {
    let name_width = identifier_width(rest);
    if name_width == 0 {
        return Group::Literal;
    }
    let Some(after_name) = rest.get(name_width..) else {
        return Group::Literal;
    };
    let Some(converted) = after_name.strip_prefix(':') else {
        if !after_name.starts_with('}') {
            return Group::Literal;
        }
        let width = name_width.saturating_add(2);
        return Group::Placeholder {
            width,
            convertor: Convertor::Text,
        };
    };
    let Some((width, name)) = converted_group(converted, name_width) else {
        return Group::Literal;
    };
    match convertor_of(name) {
        Some(convertor) => Group::Placeholder { width, convertor },
        None => Group::Unknown,
    }
}

// the width of "{name:convertor}" and the convertor's name, or None when the group is not a placeholder at all
fn converted_group(converted: &str, name_width: usize) -> Option<(usize, &str)> {
    let convertor_width = identifier_width(converted);
    let name = converted
        .get(..convertor_width)
        .filter(|_| convertor_width > 0)?;
    let after = converted.get(convertor_width..)?;
    if !after.starts_with('}') {
        return None;
    }
    Some((
        name_width.saturating_add(convertor_width).saturating_add(3),
        name,
    ))
}

fn identifier_width(text: &str) -> usize {
    let mut characters = text.chars();
    match characters.next() {
        Some(character) if character == '_' || character.is_ascii_alphabetic() => {}
        Some(_) | None => return 0,
    }
    let mut width = 1_usize;
    for character in characters {
        if character != '_' && !character.is_ascii_alphanumeric() {
            break;
        }
        width = width.saturating_add(1);
    }
    width
}

fn convertor_of(name: &str) -> Option<Convertor> {
    match name {
        "str" => Some(Convertor::Text),
        "path" => Some(Convertor::Path),
        "int" => Some(Convertor::Int),
        "float" => Some(Convertor::Float),
        "uuid" => Some(Convertor::Uuid),
        _ => None,
    }
}

fn matches_pieces(pieces: &[Piece<'_>], route: &str) -> bool {
    let mut positions = BTreeSet::from([0_usize]);
    for piece in pieces {
        let mut next = BTreeSet::new();
        for start in positions {
            let Some(rest) = route.get(start..) else {
                continue;
            };
            let Some(tail) = rest.strip_prefix(piece.literal) else {
                continue;
            };
            let after_literal = route.len().saturating_sub(tail.len());
            match piece.convertor {
                Some(convertor) => {
                    for end in after_literal..=route.len() {
                        let Some(candidate) = route.get(after_literal..end) else {
                            continue;
                        };
                        if convertor.accepts(candidate) {
                            next.insert(end);
                        }
                    }
                }
                None => {
                    next.insert(after_literal);
                }
            }
        }
        if next.is_empty() {
            return false;
        }
        positions = next;
    }
    positions.contains(&route.len())
}
