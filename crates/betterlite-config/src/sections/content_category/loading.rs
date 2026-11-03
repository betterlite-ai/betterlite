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

use super::{
    files::{CategoryFileError, load_category_file},
    model::CategoryConfig,
    paths::resolve_category_file_path,
};

#[derive(Debug, thiserror::Error)]
pub enum CategoryLoadError {
    #[error("keyword record lacks {field}")]
    MissingField { field: &'static str },
    #[error(transparent)]
    File(#[from] CategoryFileError),
}

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
    settings: &super::Settings,
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
        load_one_category(
            config,
            roots,
            data_dir,
            settings.allow_external_paths,
            &mut result,
        );
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
        match load_conditional_category(&category, config.action, config.severity_threshold, roots)
        {
            Ok(conditional) => {
                result
                    .conditional_categories
                    .insert(name.clone(), conditional);
            }
            Err(error) => {
                tracing::error!(category = name, inherit = ?category.inherit_from, %error, "failed to load conditional category");
            }
        }
    }
    let updates = add_keywords(&category.always_block_keywords, config, true, result)
        .and_then(|()| add_keywords(&category.keywords, config, false, result));
    if let Err(error) = updates {
        tracing::error!(category = name, %error, "stopped loading category keywords");
    }
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
) -> Result<(), CategoryLoadError> {
    for keyword in keywords {
        let (word, severity) = keyword_fields(keyword, always_block)?;
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
    Ok(())
}

fn keyword_fields(
    keyword: &BTreeMap<String, String>,
    always_block: bool,
) -> Result<(&str, &str), CategoryLoadError> {
    let word = keyword
        .get("keyword")
        .ok_or(CategoryLoadError::MissingField { field: "keyword" })?;
    let severity = keyword
        .get("severity")
        .map(String::as_str)
        .or_else(|| always_block.then_some("high"))
        .ok_or(CategoryLoadError::MissingField { field: "severity" })?;
    Ok((word, severity))
}

const fn severity_name(severity: ContentFilterSeverity) -> &'static str {
    match severity {
        ContentFilterSeverity::Low => "low",
        ContentFilterSeverity::Medium => "medium",
        ContentFilterSeverity::High => "high",
    }
}

pub fn load_conditional_category(
    category: &CategoryConfig,
    action: ContentFilterAction,
    threshold: ContentFilterSeverity,
    roots: &[PathBuf],
) -> Result<ConditionalCategoryConfig, CategoryLoadError> {
    let mut block_words = Vec::new();
    if let Some(inherit) = &category.inherit_from {
        let base = inherit.trim_end_matches(".yaml").trim_end_matches(".json");
        append_inherited_words(base, threshold, roots, &mut block_words)?;
    }
    block_words.extend(category.additional_block_words.iter().cloned());
    Ok(ConditionalCategoryConfig {
        identifier_words: category.identifier_words.clone(),
        block_words,
        action,
        severity: "high".to_owned(),
    })
}

fn append_inherited_words(
    base: &str,
    threshold: ContentFilterSeverity,
    roots: &[PathBuf],
    words: &mut Vec<String>,
) -> Result<(), CategoryLoadError> {
    let root_refs: Vec<_> = roots.iter().map(PathBuf::as_path).collect();
    let Some(path) = find_category_file(base, &root_refs) else {
        tracing::warn!(inherit = base, ?roots, "inherited category file not found");
        return Ok(());
    };
    let inherited = load_category_file(&path)?;
    for keyword in inherited.keywords {
        let (word, severity) = keyword_fields(&keyword, false)?;
        if should_apply_severity(severity, severity_name(threshold)) {
            words.push(word.to_lowercase());
        }
    }
    Ok(())
}
