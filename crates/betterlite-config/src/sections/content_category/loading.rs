use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::sections::{
    content_filter_catalog::{categories::find_category_file, roots::CATEGORIES_DIR},
    content_filter_records::{
        ContentFilterAction, ContentFilterCategoryConfig, ContentFilterSeverity,
        category_types::ConditionalCategoryConfig,
    },
};

use super::{files::load_category_file, model::CategoryConfig, paths::resolve_category_file_path};

#[derive(Debug, Default)]
pub struct CategoryLoadResult {
    pub loaded_categories: BTreeMap<String, CategoryConfig>,
    pub category_keywords: BTreeMap<String, (String, String, ContentFilterAction)>,
    pub always_block_category_keywords: BTreeMap<String, (String, String, ContentFilterAction)>,
    pub conditional_categories: BTreeMap<String, ConditionalCategoryConfig>,
}

#[must_use]
pub fn should_apply_severity(severity: &str, threshold: &str) -> bool {
    let rank = |value, fallback| match value {
        "low" => 0_u8,
        "medium" => 1_u8,
        "high" => 2_u8,
        _ => fallback,
    };
    rank(severity, 0_u8) >= rank(threshold, 1_u8)
}

pub fn load_categories(
    categories: &[ContentFilterCategoryConfig],
    roots: &[PathBuf],
    data_dir: &Path,
    allow_external: bool,
) -> CategoryLoadResult {
    let mut result = CategoryLoadResult::default();
    for config in categories {
        if !valid_category_name(&config.category) {
            tracing::warn!(
                category = config.category,
                "category name missing or invalid; skipping"
            );
            continue;
        }
        if !config.enabled {
            continue;
        }
        load_one_category(config, roots, data_dir, allow_external, &mut result);
    }
    result
}

fn valid_category_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn load_one_category(
    config: &ContentFilterCategoryConfig,
    roots: &[PathBuf],
    data_dir: &Path,
    allow_external: bool,
    result: &mut CategoryLoadResult,
) {
    let name = &config.category;
    let resolved_path = category_path(config, roots, data_dir, allow_external);
    let category_path = match resolved_path {
        Ok(candidate) if candidate.is_file() => candidate,
        Ok(candidate) => {
            tracing::warn!(category = name, path = %candidate.display(), "category file not found; skipping");
            return;
        }
        Err(error) => {
            tracing::warn!(category = name, %error, "invalid category file path; skipping");
            return;
        }
    };
    match load_category_file(&category_path) {
        Ok(category) => load_category_result(config, category, roots, result),
        Err(error) => tracing::error!(category = name, %error, "failed to load category; skipping"),
    }
}

fn category_path(
    config: &ContentFilterCategoryConfig,
    roots: &[PathBuf],
    data_dir: &Path,
    allow_external: bool,
) -> Result<PathBuf, super::paths::CategoryPathError> {
    if let Some(file) = config.category_file.as_deref() {
        return resolve_category_file_path(Path::new(file), roots, data_dir, allow_external);
    }
    let root_refs: Vec<_> = roots.iter().map(PathBuf::as_path).collect();
    Ok(find_category_file(&config.category, &root_refs)
        .unwrap_or_else(|| PathBuf::from(CATEGORIES_DIR).join(format!("{}.yaml", config.category))))
}

fn load_category_result(
    config: &ContentFilterCategoryConfig,
    category: CategoryConfig,
    roots: &[PathBuf],
    result: &mut CategoryLoadResult,
) {
    let name = &config.category;
    if conditional_category_applies(&category) {
        let conditional =
            load_conditional_category(&category, config.action, config.severity_threshold, roots);
        result
            .conditional_categories
            .insert(name.clone(), conditional);
    }
    add_keywords(&category.always_block_keywords, config, true, result);
    add_keywords(&category.keywords, config, false, result);
    result.loaded_categories.insert(name.clone(), category);
}

const fn conditional_category_applies(category: &CategoryConfig) -> bool {
    !category.identifier_words.is_empty()
        && (category.inherit_from.is_some() || !category.additional_block_words.is_empty())
}

fn add_keywords(
    keywords: &[BTreeMap<String, String>],
    config: &ContentFilterCategoryConfig,
    always_block: bool,
    result: &mut CategoryLoadResult,
) {
    for keyword in keywords {
        let Some(word) = keyword.get("keyword") else {
            continue;
        };
        let severity = keyword.get("severity").map_or("high", String::as_str);
        if !should_apply_severity(severity, severity_name(config.severity_threshold)) {
            continue;
        }
        let target = if always_block {
            &mut result.always_block_category_keywords
        } else {
            &mut result.category_keywords
        };
        target.insert(
            word.to_lowercase(),
            (config.category.clone(), severity.to_owned(), config.action),
        );
    }
}

const fn severity_name(severity: ContentFilterSeverity) -> &'static str {
    match severity {
        ContentFilterSeverity::Low => "low",
        ContentFilterSeverity::Medium => "medium",
        ContentFilterSeverity::High => "high",
    }
}

#[must_use]
pub fn load_conditional_category(
    category: &CategoryConfig,
    action: ContentFilterAction,
    threshold: ContentFilterSeverity,
    roots: &[PathBuf],
) -> ConditionalCategoryConfig {
    let mut block_words = category.additional_block_words.clone();
    if let Some(inherit) = &category.inherit_from {
        let base = inherit.trim_end_matches(".yaml").trim_end_matches(".json");
        append_inherited_words(base, threshold, roots, &mut block_words);
    }
    ConditionalCategoryConfig {
        identifier_words: category.identifier_words.clone(),
        block_words,
        action,
        severity: "high".to_owned(),
    }
}

fn append_inherited_words(
    base: &str,
    threshold: ContentFilterSeverity,
    roots: &[PathBuf],
    words: &mut Vec<String>,
) {
    let root_refs: Vec<_> = roots.iter().map(PathBuf::as_path).collect();
    let Some(path) = find_category_file(base, &root_refs) else {
        return;
    };
    let inherited = match load_category_file(&path) {
        Ok(category) => category,
        Err(error) => {
            tracing::warn!(inherit = base, %error, "failed to load inherited category");
            return;
        }
    };
    for keyword in inherited.keywords {
        let Some(word) = keyword.get("keyword") else {
            continue;
        };
        let Some(severity) = keyword.get("severity") else {
            continue;
        };
        if should_apply_severity(severity, severity_name(threshold)) {
            words.push(word.to_lowercase());
        }
    }
}
