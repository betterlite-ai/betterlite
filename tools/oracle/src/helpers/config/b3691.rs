use std::fs;
use std::path::{Path, PathBuf};

use betterlite_config::sections::content_filter_catalog::categories::available_content_categories;
use betterlite_config::sections::content_filter_catalog::patterns::{
    PatternCatalogError, get_compiled_pattern, get_pattern_catalog,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Catalog,
    Compile { name: String, texts: Vec<String> },
    Categories { roots: Vec<Vec<CategoryFile>> },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CategoryFile {
    filename: String,
    content: String,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Catalog => catalog(),
            Input::Compile { name, texts } => compile(name, texts),
            Input::Categories { roots } => categories(roots),
        }
    }
}

fn catalog() -> HelperResult {
    let catalog = get_pattern_catalog().map_err(pattern_error)?;
    serde_json::to_value(catalog).map_err(|error| {
        Box::new(HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

fn compile(name: &str, texts: &[String]) -> HelperResult {
    let regex = get_compiled_pattern(name).map_err(pattern_error)?;
    let mut matches = Vec::with_capacity(texts.len());
    for text in texts {
        matches.push(regex.is_match(text).map_err(|error| {
            Box::new(HelperError {
                class: "RegexError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?);
    }
    Ok(json!(matches))
}

fn categories(roots: &[Vec<CategoryFile>]) -> HelperResult {
    let mut temporary_roots = Vec::with_capacity(roots.len());
    let mut warning_names = Vec::new();
    for files in roots {
        let temporary =
            TempDir::new().map_err(|error| helper_error("IOError", error.to_string()))?;
        let categories_dir = temporary.path().join("categories");
        fs::create_dir_all(&categories_dir)
            .map_err(|error| helper_error("IOError", error.to_string()))?;
        for file in files {
            if Path::new(&file.filename)
                .file_name()
                .and_then(|name| name.to_str())
                != Some(file.filename.as_str())
            {
                return Err(helper_error(
                    "ValueError",
                    "fixture filename must be a basename".to_owned(),
                ));
            }
            fs::write(categories_dir.join(&file.filename), &file.content)
                .map_err(|error| helper_error("IOError", error.to_string()))?;
            if file.filename.ends_with(".yaml") || file.filename.ends_with(".yml") {
                if serde_saphyr::from_str::<Value>(&file.content).is_err() {
                    warning_names.push(file.filename.clone());
                }
            }
        }
        temporary_roots.push(temporary);
    }
    let root_paths: Vec<PathBuf> = temporary_roots
        .iter()
        .map(|root| root.path().to_path_buf())
        .collect();
    let root_refs: Vec<&Path> = root_paths.iter().map(PathBuf::as_path).collect();
    let entries = available_content_categories(&root_refs);
    let categories: Vec<Value> = entries
        .iter()
        .filter_map(|entry| serde_json::to_value(entry).ok())
        .collect();
    Ok(json!({"categories": categories, "warnings": warning_names}))
}

fn pattern_error(error: PatternCatalogError) -> Box<HelperError> {
    let class = match &error {
        PatternCatalogError::UnknownPattern { .. } => "ValueError",
        PatternCatalogError::Compile { .. } => "RegexError",
        PatternCatalogError::Read { .. } | PatternCatalogError::Parse { .. } => "PatternError",
    };
    helper_error(class, error.to_string())
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

mod tests;
