use std::collections::HashMap;
use std::fs;
use std::path::{self, Component, Path, PathBuf};

use unicode_general_category::{GeneralCategory, get_general_category};

use crate::sections::content_filter_records::ContentFilterAction;

const CASE_IGNORABLE_PUNCTUATION: &[(char, char)] = &[
    ('\u{0027}', '\u{0027}'),
    ('\u{002e}', '\u{002e}'),
    ('\u{003a}', '\u{003a}'),
    ('\u{005e}', '\u{005e}'),
    ('\u{0060}', '\u{0060}'),
    ('\u{00b7}', '\u{00b7}'),
    ('\u{0387}', '\u{0387}'),
    ('\u{055f}', '\u{055f}'),
    ('\u{05f4}', '\u{05f4}'),
    ('\u{0897}', '\u{0897}'),
    ('\u{2018}', '\u{2019}'),
    ('\u{2024}', '\u{2024}'),
    ('\u{2027}', '\u{2027}'),
    ('\u{fe13}', '\u{fe13}'),
    ('\u{fe52}', '\u{fe52}'),
    ('\u{fe55}', '\u{fe55}'),
    ('\u{ff07}', '\u{ff07}'),
    ('\u{ff0e}', '\u{ff0e}'),
    ('\u{ff1a}', '\u{ff1a}'),
    ('\u{10d4e}', '\u{10d4e}'),
    ('\u{10d69}', '\u{10d6d}'),
    ('\u{10d6f}', '\u{10d6f}'),
    ('\u{10efc}', '\u{10efc}'),
    ('\u{113bb}', '\u{113c0}'),
    ('\u{113ce}', '\u{113ce}'),
    ('\u{113d0}', '\u{113d0}'),
    ('\u{113d2}', '\u{113d2}'),
    ('\u{113e1}', '\u{113e2}'),
    ('\u{11f5a}', '\u{11f5a}'),
    ('\u{1611e}', '\u{16129}'),
    ('\u{1612d}', '\u{1612f}'),
    ('\u{16d40}', '\u{16d42}'),
    ('\u{16d6b}', '\u{16d6c}'),
    ('\u{1e5ee}', '\u{1e5ef}'),
];

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CategoryCatalogEntry {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub default_action: ContentFilterAction,
}

#[must_use]
pub fn category_dirs(roots: &[&Path]) -> Vec<PathBuf> {
    roots
        .iter()
        .map(|root| root.join("categories"))
        .filter(|dir| dir.is_dir())
        .collect()
}

#[must_use]
pub fn find_category_file(category_name: &str, roots: &[&Path]) -> Option<PathBuf> {
    for dir in category_dirs(roots) {
        for extension in ["yaml", "json"] {
            let candidate = join_within(&dir, &format!("{category_name}.{extension}"));
            if candidate.as_ref().is_some_and(|path| path.is_file()) {
                return candidate;
            }
        }
    }
    None
}

#[must_use]
pub fn category_files(roots: &[&Path]) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    for dir in category_dirs(roots) {
        let Ok(read) = fs::read_dir(&dir) else {
            continue;
        };
        let mut names: Vec<String> = read
            .filter_map(|entry| {
                let found = entry.ok()?;
                found.file_name().into_string().ok()
            })
            .collect();
        names.sort();
        files.extend(names.into_iter().map(|name| (dir.clone(), name)));
    }
    files
}

#[must_use]
pub fn available_content_categories(roots: &[&Path]) -> Vec<CategoryCatalogEntry> {
    let mut by_name: HashMap<String, CategoryCatalogEntry> = HashMap::new();
    for (dir, filename) in category_files(roots) {
        if let Some(entry) = category_entry(&dir, &filename) {
            by_name.entry(entry.name.clone()).or_insert(entry);
        }
    }
    let mut entries: Vec<CategoryCatalogEntry> = by_name.into_values().collect();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries
}

#[derive(Debug, Default, serde::Deserialize)]
struct CategoryFileRaw {
    category_name: Option<String>,
    display_name: Option<String>,
    description: Option<String>,
    default_action: Option<ContentFilterAction>,
}

#[must_use]
pub fn category_entry(dir: &Path, filename: &str) -> Option<CategoryCatalogEntry> {
    let extension = Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str());
    match extension {
        Some("yaml" | "yml") => yaml_category_entry(dir, filename),
        Some("json") => {
            let stem = filename.strip_suffix(".json")?;
            Some(json_category_entry(stem))
        }
        _ => None,
    }
}

fn yaml_category_entry(dir: &Path, filename: &str) -> Option<CategoryCatalogEntry> {
    let content = fs::read_to_string(dir.join(filename))
        .inspect_err(|error| tracing::warn!(filename, %error, "failed to load category file"))
        .ok()?;
    let raw: CategoryFileRaw = serde_saphyr::from_str(&content)
        .inspect_err(|error| tracing::warn!(filename, %error, "failed to load category file"))
        .ok()?;
    let name = raw.category_name?;
    let display_name = raw
        .display_name
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| title_case(&name));
    Some(CategoryCatalogEntry {
        name,
        display_name,
        description: raw.description.unwrap_or_default(),
        default_action: raw.default_action.unwrap_or(ContentFilterAction::Block),
    })
}

fn json_category_entry(stem: &str) -> CategoryCatalogEntry {
    if stem == "harm_toxic_abuse" {
        return CategoryCatalogEntry {
            name: stem.to_owned(),
            display_name: "Harmful Toxic Abuse".to_owned(),
            description: "Detects harmful, toxic, or abusive language and content".to_owned(),
            default_action: ContentFilterAction::Block,
        };
    }
    let display_name = title_case(stem);
    CategoryCatalogEntry {
        name: stem.to_owned(),
        description: format!("Content category: {display_name}"),
        display_name,
        default_action: ContentFilterAction::Block,
    }
}

fn title_case(value: &str) -> String {
    let mut titled = String::with_capacity(value.len());
    let mut capitalize = true;
    let characters: Vec<char> = value.chars().collect();
    for (position, character) in characters.iter().copied().enumerate() {
        let lowercase_mapping: String = character.to_lowercase().collect();
        if character == '_' {
            titled.push(' ');
            capitalize = true;
        } else if is_cased(character) {
            if capitalize {
                titled.extend(
                    unicode_titlecase::to_titlecase(character)
                        .into_iter()
                        .take_while(|titlecase| *titlecase != '\0'),
                );
                capitalize = false;
            } else if character == '\u{03a3}' && is_final_sigma(&characters, position) {
                titled.push('\u{03c2}');
            } else {
                titled.push_str(&lowercase_mapping);
            }
        } else {
            titled.push(character);
            capitalize = true;
        }
    }
    titled
}

fn is_cased(character: char) -> bool {
    let lowercase_mapping: String = character.to_lowercase().collect();
    character.is_lowercase()
        || character.is_uppercase()
        || lowercase_mapping != character.to_string()
}

fn is_ignorable_category(character: char) -> bool {
    matches!(
        get_general_category(character),
        GeneralCategory::NonspacingMark
            | GeneralCategory::EnclosingMark
            | GeneralCategory::Format
            | GeneralCategory::ModifierLetter
            | GeneralCategory::ModifierSymbol
    )
}

fn is_case_ignorable_punctuation(character: char) -> bool {
    CASE_IGNORABLE_PUNCTUATION
        .iter()
        .any(|(start, end)| *start <= character && character <= *end)
}

fn is_final_sigma(characters: &[char], position: usize) -> bool {
    let preceding_cased = characters
        .iter()
        .take(position)
        .rev()
        .find(|character| {
            !is_ignorable_category(**character) && !is_case_ignorable_punctuation(**character)
        })
        .is_some_and(|character| is_cased(*character));
    let following_cased = characters
        .iter()
        .enumerate()
        .skip(position)
        .skip(1)
        .map(|(_, character)| character)
        .find(|character| {
            !is_ignorable_category(**character) && !is_case_ignorable_punctuation(**character)
        })
        .is_some_and(|character| is_cased(*character));
    preceding_cased && !following_cased
}

// Mirrors the lexical half of betterlite_server::assets::paths::join_within, without symlink resolution.
fn join_within(dir: &Path, filename: &str) -> Option<PathBuf> {
    if filename.contains('\0') {
        return None;
    }
    let base = lexically_normalize(&path::absolute(dir).ok()?);
    let normalized = lexically_normalize(&base.join(filename));
    if normalized == base || !normalized.starts_with(&base) {
        return None;
    }
    Some(normalized)
}

fn lexically_normalize(path: &Path) -> PathBuf {
    let mut stack: Vec<Component<'_>> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(stack.last(), Some(Component::Normal(_))) {
                    stack.pop();
                }
            }
            other @ (Component::Prefix(_) | Component::RootDir | Component::Normal(_)) => {
                stack.push(other);
            }
        }
    }
    stack.into_iter().collect()
}

mod tests;
