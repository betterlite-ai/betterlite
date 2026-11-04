use std::collections::BTreeSet;

#[must_use]
pub fn transcribe_operations() -> BTreeSet<&'static str> {
    include_str!("../data/operations.txt")
        .lines()
        .filter(|operation| !operation.is_empty())
        .collect()
}

mod tests;
