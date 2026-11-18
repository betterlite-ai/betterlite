use super::Literal;

// a dict key or set element that is a list, set or dict makes Python's evaluation raise TypeError
#[must_use]
pub fn has_unhashable_member(value: &Literal) -> bool {
    match value {
        Literal::Dict(entries) => entries
            .iter()
            .any(|(key, item)| unhashable(key) || has_unhashable_member(item)),
        Literal::Set(items) => items.iter().any(unhashable),
        Literal::List(items) | Literal::Tuple(items) => items.iter().any(has_unhashable_member),
        Literal::None
        | Literal::Bool(_)
        | Literal::Number
        | Literal::Ellipsis
        | Literal::Bytes
        | Literal::Str(_) => false,
    }
}

// a tuple's hash covers its elements, and the scalar values hash on their own
fn unhashable(value: &Literal) -> bool {
    if let Literal::Tuple(items) = value {
        return items.iter().any(unhashable);
    }
    matches!(value, Literal::List(_) | Literal::Set(_) | Literal::Dict(_))
}
