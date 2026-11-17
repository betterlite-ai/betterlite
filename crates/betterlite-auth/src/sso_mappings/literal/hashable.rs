use super::Literal;

// a dict key or set element that is a list, set or dict makes Python's evaluation raise TypeError
#[must_use]
pub fn has_unhashable_member(value: &Literal) -> bool {
    match value {
        Literal::Dict(entries) => entries
            .iter()
            .any(|(key, item)| !is_hashable(key) || has_unhashable_member(item)),
        Literal::Set(items) => items.iter().any(|item| !is_hashable(item)),
        Literal::List(items) | Literal::Tuple(items) => items.iter().any(has_unhashable_member),
        Literal::None
        | Literal::Bool(_value)
        | Literal::Number
        | Literal::Ellipsis
        | Literal::Bytes
        | Literal::Str(_text) => false,
    }
}

fn is_hashable(value: &Literal) -> bool {
    match value {
        Literal::List(_items) | Literal::Set(_items) | Literal::Dict(_entries) => false,
        Literal::Tuple(items) => items.iter().all(is_hashable),
        Literal::None
        | Literal::Bool(_value)
        | Literal::Number
        | Literal::Ellipsis
        | Literal::Bytes
        | Literal::Str(_text) => true,
    }
}
