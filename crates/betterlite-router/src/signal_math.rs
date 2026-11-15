#![expect(
    clippy::float_arithmetic,
    reason = "pinned statistical routing arithmetic"
)]

use std::collections::BTreeSet;
use std::mem::take;

// LiteLLM splits text on every run outside [A-Za-z0-9] and lowercases each run into a set
#[must_use]
pub fn tokens(text: Option<&str>) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut current = String::new();
    for character in text.unwrap_or_default().chars() {
        if character.is_ascii_alphanumeric() {
            current.push(character.to_ascii_lowercase());
            continue;
        }
        if !current.is_empty() {
            found.insert(take(&mut current));
        }
    }
    if !current.is_empty() {
        found.insert(current);
    }
    found
}

#[must_use]
pub fn jaccard(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    let union = left.union(right).count();
    if union == 0 {
        return 0.0;
    }
    ratio(left.intersection(right).count(), union)
}

fn ratio(part: usize, whole: usize) -> f64 {
    let part_as_f64 = f64::from(u32::try_from(part).unwrap_or(u32::MAX));
    let whole_as_f64 = f64::from(u32::try_from(whole).unwrap_or(u32::MAX));
    part_as_f64 / whole_as_f64
}
