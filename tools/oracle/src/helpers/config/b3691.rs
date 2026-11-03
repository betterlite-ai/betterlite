use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use betterlite_config::sections::content_filter_catalog::categories::available_content_categories;
use betterlite_config::sections::content_filter_catalog::patterns::{
    PatternCatalogError, get_compiled_pattern, get_pattern_catalog,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tempfile::TempDir;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

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
    let catalog = get_pattern_catalog().map_err(|error| {
        let class = pattern_error_class(&error);
        Box::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
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
    let regex = get_compiled_pattern(name).map_err(|error| {
        let class = pattern_error_class(&error);
        Box::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
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
    let temporary_roots = write_category_roots(roots)?;
    let root_paths: Vec<PathBuf> = temporary_roots
        .iter()
        .map(|root| root.path().to_path_buf())
        .collect();
    let root_refs: Vec<&Path> = root_paths.iter().map(PathBuf::as_path).collect();
    let warnings = WarningCapture::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    let entries = with_default(subscriber, || available_content_categories(&root_refs));
    let warning_entries = warnings
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let categories: Vec<Value> = entries
        .iter()
        .filter_map(|entry| serde_json::to_value(entry).ok())
        .collect();
    Ok(json!({"categories": categories, "warnings": warning_entries}))
}

fn write_category_roots(roots: &[Vec<CategoryFile>]) -> Result<Vec<TempDir>, Box<HelperError>> {
    let mut temporary_roots = Vec::with_capacity(roots.len());
    for files in roots {
        temporary_roots.push(write_category_root(files)?);
    }
    Ok(temporary_roots)
}

fn write_category_root(files: &[CategoryFile]) -> Result<TempDir, Box<HelperError>> {
    let (temporary, categories_dir) = create_category_directory()?;
    for file in files {
        if Path::new(&file.filename)
            .file_name()
            .and_then(|name| name.to_str())
            != Some(file.filename.as_str())
        {
            return Err(Box::new(HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "fixture filename must be a basename".to_owned(),
            }));
        }
        fs::write(categories_dir.join(&file.filename), &file.content).map_err(|error| {
            Box::new(HelperError {
                class: "IOError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
    }
    Ok(temporary)
}

fn create_category_directory() -> Result<(TempDir, PathBuf), Box<HelperError>> {
    let temporary = TempDir::new().map_err(|error| {
        Box::new(HelperError {
            class: "IOError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    let categories_dir = temporary.path().join("categories");
    fs::create_dir_all(&categories_dir).map_err(|error| {
        Box::new(HelperError {
            class: "IOError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })?;
    Ok((temporary, categories_dir))
}

const fn pattern_error_class(error: &PatternCatalogError) -> &'static str {
    match error {
        PatternCatalogError::UnknownPattern { .. } => "ValueError",
        PatternCatalogError::Compile { .. } => "RegexError",
        PatternCatalogError::Parse { .. } => "PatternError",
    }
}

#[derive(Clone, Default)]
struct WarningCapture(Arc<Mutex<Vec<CategoryWarning>>>);

#[derive(Clone, serde::Serialize)]
struct CategoryWarning {
    filename: String,
    error: bool,
}

#[derive(Default)]
struct WarningFields(BTreeMap<String, String>);

impl Visit for WarningFields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        let rendered = format!("{value:?}");
        self.0.insert(
            field.name().to_owned(),
            rendered.trim_matches('"').to_owned(),
        );
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

impl<Inner: Subscriber> Layer<Inner> for WarningCapture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != tracing::Level::WARN {
            return;
        }
        let mut fields = WarningFields::default();
        event.record(&mut fields);
        let warning = CategoryWarning {
            filename: fields.0.get("filename").cloned().unwrap_or_default(),
            error: fields.0.get("error").is_some_and(|error| !error.is_empty()),
        };
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(warning);
    }
}

mod tests;
