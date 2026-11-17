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
        _scalar => false,
    }
}

fn unhashable(value: &Literal) -> bool {
    match value {
        Literal::List(_) | Literal::Set(_) | Literal::Dict(_) => true,
        Literal::Tuple(items) => items.iter().any(unhashable),
        _scalar => false,
    }
}
