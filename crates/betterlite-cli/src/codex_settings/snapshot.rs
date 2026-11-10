use betterlite_logging::python_json::dumps;
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use toml_edit::{Decor, DocumentMut, Item, Table, TableLike, TomlError};

use super::canonical;

const VALUE_KEY: &str = "value";

#[derive(Debug)]
pub struct Snapshot {
    text: String,
    item: Item,
}

impl AsRef<str> for Snapshot {
    fn as_ref(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("the saved value is not valid TOML")]
    Toml {
        #[source]
        source: TomlError,
    },
    #[error("the saved value does not hold exactly one `value` key")]
    Shape,
}

impl Snapshot {
    #[must_use]
    pub fn from_item(item: &Item) -> Self {
        let mut document = DocumentMut::new();
        document.as_table_mut().insert(VALUE_KEY, item.clone());
        Self {
            text: document.to_string(),
            item: item.clone(),
        }
    }

    pub fn parse(text: &str) -> Result<Self, SnapshotError> {
        let mut document = text
            .parse::<DocumentMut>()
            .map_err(|source| SnapshotError::Toml { source })?;
        let table = document.as_table_mut();
        if table.len() != 1 || !table.contains_key(VALUE_KEY) {
            return Err(SnapshotError::Shape);
        }
        let item = table.remove(VALUE_KEY).ok_or(SnapshotError::Shape)?;
        Ok(Self {
            text: text.to_owned(),
            item,
        })
    }
}

impl PartialEq for Snapshot {
    fn eq(&self, other: &Self) -> bool {
        dumps(&canonical::to_json(&self.item)) == dumps(&canonical::to_json(&other.item))
    }
}

impl Eq for Snapshot {}

#[must_use]
pub fn fingerprint(snapshot: Option<&Snapshot>) -> String {
    let canonical = snapshot.map_or_else(
        || "missing".to_owned(),
        |found| format!("{{\"value\": {}}}", dumps(&canonical::to_json(&found.item))),
    );
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

#[must_use]
pub fn snapshot(document: &DocumentMut, path: &str) -> Option<Snapshot> {
    let (section, key) = path.rsplit_once('.').unwrap_or(("", path));
    let parent: &dyn TableLike = if section.is_empty() {
        document.as_table()
    } else {
        document.as_table().get(section)?.as_table_like()?
    };
    parent.get(key).map(Snapshot::from_item)
}

#[must_use]
pub fn with_value(document: &DocumentMut, path: &str, snapshot: Option<&Snapshot>) -> DocumentMut {
    let (section, key) = path.rsplit_once('.').unwrap_or(("", path));
    let mut updated = document.clone();
    if !section.is_empty() && !updated.as_table().contains_key(section) {
        if let Some(replacement) = snapshot {
            insert_section(&mut updated, section, key, replacement);
        }
        return updated;
    }
    let Some(parent) = section_table(&mut updated, section) else {
        return updated;
    };
    match snapshot {
        Some(replacement) => replace_value(parent, key, replacement),
        None => drop(parent.remove(key)),
    }
    updated
}

fn section_table<'document>(
    document: &'document mut DocumentMut,
    section: &str,
) -> Option<&'document mut dyn TableLike> {
    if section.is_empty() {
        return Some(document.as_table_mut());
    }
    document
        .as_table_mut()
        .get_mut(section)?
        .as_table_like_mut()
}

fn insert_section(document: &mut DocumentMut, section: &str, key: &str, snapshot: &Snapshot) {
    let mut table = Table::new();
    table.insert(key, snapshot.item.clone());
    drop(document.as_table_mut().insert(section, Item::Table(table)));
}

fn replace_value(parent: &mut dyn TableLike, key: &str, snapshot: &Snapshot) {
    let mut replacement = snapshot.item.clone();
    if let Some(decor) = parent.get(key).and_then(decor_of).cloned() {
        set_decor(&mut replacement, decor);
    }
    // Table::insert resets the key's own decor, which carries the comment above the line
    let previous_key = parent
        .key_mut(key)
        .map(|current| (current.leaf_decor().clone(), current.dotted_decor().clone()));
    drop(parent.insert(key, replacement));
    if let (Some((leaf, dotted)), Some(mut current_key)) = (previous_key, parent.key_mut(key)) {
        *current_key.leaf_decor_mut() = leaf;
        *current_key.dotted_decor_mut() = dotted;
    }
}

fn decor_of(item: &Item) -> Option<&Decor> {
    match item {
        Item::Value(value) => Some(value.decor()),
        Item::Table(table) => Some(table.decor()),
        Item::ArrayOfTables(_) | Item::None => None,
    }
}

fn set_decor(item: &mut Item, decor: Decor) {
    match item {
        Item::Value(value) => *value.decor_mut() = decor,
        Item::Table(table) => *table.decor_mut() = decor,
        Item::ArrayOfTables(_) | Item::None => {}
    }
}

mod tests;
