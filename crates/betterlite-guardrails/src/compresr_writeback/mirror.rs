use std::collections::BTreeMap;

use super::results::Replacements;

// Mirrors the compression into texts by value, replacing a text only when it has one compression and every occurrence was compressed
#[must_use]
pub(super) fn mirror_texts_channel(
    texts: Option<&[String]>,
    replacements: &Replacements,
) -> Option<Vec<String>> {
    let input = texts?;
    if replacements.by_text.is_empty() {
        return None;
    }
    let mut occurrences: BTreeMap<&str, usize> = BTreeMap::new();
    for text in input {
        let count = occurrences.entry(text.as_str()).or_default();
        *count = count.saturating_add(1);
    }
    let safe: BTreeMap<&str, &str> = replacements
        .by_text
        .iter()
        .filter(|(text, _)| {
            !replacements.ambiguous.contains(*text)
                && occurrences.get(text.as_str()) == replacements.counts.get(*text)
        })
        .map(|(text, replacement)| (text.as_str(), replacement.as_str()))
        .collect();
    if safe.is_empty() {
        return None;
    }
    Some(
        input
            .iter()
            .map(|text| {
                safe.get(text.as_str())
                    .map_or_else(|| text.clone(), |replacement| (*replacement).to_owned())
            })
            .collect(),
    )
}
