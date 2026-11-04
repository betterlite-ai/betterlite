use std::collections::BTreeSet;
use std::sync::OnceLock;

static OPERATIONS: &str = include_str!("../data/operations.txt");
static CACHED_OPERATIONS: OnceLock<BTreeSet<&'static str>> = OnceLock::new();

#[must_use]
pub fn transcribe_operations() -> &'static BTreeSet<&'static str> {
    match CACHED_OPERATIONS.get() {
        Some(operations) => operations,
        None => CACHED_OPERATIONS.get_or_init(|| OPERATIONS.lines().collect()),
    }
}
