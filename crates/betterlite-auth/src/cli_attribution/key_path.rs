use std::collections::BTreeMap;
use std::mem;

use betterlite_db::budget_permission_rows::JsonData;

// Python turns an escaped dot into NUL, splits on the remaining dots and turns every NUL left in a part into a dot.
pub(crate) fn split_key_path(key_path: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut characters = key_path.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '.' => parts.push(mem::take(&mut part)),
            '\\' if characters.peek() == Some(&'.') => {
                part.push('.');
                characters.next();
            }
            other => part.push(other),
        }
    }
    parts.push(part);
    parts
        .into_iter()
        .map(|piece| piece.replace('\0', "."))
        .collect()
}

pub(crate) fn value_at<'a>(
    metadata: &'a BTreeMap<String, JsonData>,
    key_path: &[String],
) -> Option<&'a JsonData> {
    let mut parts = key_path.iter();
    let mut current = metadata.get(parts.next()?)?;
    for part in parts {
        let JsonData::Object(children) = current else {
            return None;
        };
        current = children.get(part)?;
    }
    Some(current)
}

pub(crate) fn insert_at(
    metadata: &mut BTreeMap<String, JsonData>,
    key_path: &[String],
    value: JsonData,
) {
    let Some((key, parents)) = key_path.split_last() else {
        return;
    };
    let mut current = metadata;
    for parent in parents {
        let entry = current
            .entry(parent.clone())
            .or_insert_with(|| JsonData::Object(BTreeMap::new()));
        if !matches!(entry, JsonData::Object(_)) {
            *entry = JsonData::Object(BTreeMap::new());
        }
        let JsonData::Object(children) = entry else {
            // the entry was just replaced with an object
            return;
        };
        current = children;
    }
    current.insert(key.clone(), value);
}
