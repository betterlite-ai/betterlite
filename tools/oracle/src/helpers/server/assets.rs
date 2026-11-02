mod fixture;

use std::cell::Cell;
use std::path::Path;

use betterlite_server::assets::images::{self, AssetLogger};
use betterlite_server::assets::paths::{self, FilenameError, PathError};
use fixture::Entry;
use soft_canonicalize::soft_canonicalize;
use tempfile::TempDir;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    SafeJoin {
        #[serde(default)]
        fixture: Vec<Entry>,
        parts: Vec<String>,
    },
    TrySafeJoin {
        #[serde(default)]
        fixture: Vec<Entry>,
        parts: Vec<String>,
    },
    IsWithin {
        #[serde(default)]
        fixture: Vec<Entry>,
        path: String,
    },
    JoinWithin {
        base_dir: String,
        parts: Vec<String>,
    },
    SafeFilename {
        filename: String,
    },
    DetectLocalImageMediaType {
        header_hex: String,
    },
    ResolveLocalImage {
        #[serde(default)]
        fixture: Vec<Entry>,
        candidate: String,
    },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::SafeJoin { fixture, parts } => run_safe_join(fixture, parts, true),
            Input::TrySafeJoin { fixture, parts } => run_safe_join(fixture, parts, false),
            Input::IsWithin { fixture, path } => run_is_within(fixture, path),
            Input::JoinWithin { base_dir, parts } => run_join_within(base_dir, parts),
            Input::SafeFilename { filename } => {
                Ok(serde_json::Value::String(paths::safe_filename(filename)?))
            }
            Input::DetectLocalImageMediaType { header_hex } => run_detect(header_hex),
            Input::ResolveLocalImage { fixture, candidate } => run_resolve(fixture, candidate),
        }
    }
}

fn prepare_base(
    entries: &[Entry],
) -> Result<(TempDir, std::path::PathBuf), Box<crate::helpers::HelperError>> {
    let root = TempDir::new()?;
    let base = root.path().join("base");
    std::fs::create_dir_all(&base)?;
    fixture::materialize(&base, entries)?;
    Ok((root, base))
}

fn run_safe_join(
    entries: &[Entry],
    parts: &[String],
    strict: bool,
) -> crate::helpers::HelperResult {
    let (_root, base) = prepare_base(entries)?;
    let canonical_base = soft_canonicalize(&base)?;
    let parts_ref: Vec<&str> = parts.iter().map(String::as_str).collect();
    let resolved = if strict {
        paths::safe_join(&base, &parts_ref)?
    } else {
        match paths::try_safe_join(&base, &parts_ref)? {
            Some(path) => path,
            None => return Ok(serde_json::Value::Null),
        }
    };
    let relative = fixture::relative_to(&canonical_base, &resolved)
        .ok_or_else(|| Invalid("resolved path left the fixture root".to_owned()))?;
    Ok(serde_json::Value::String(relative))
}

fn run_is_within(entries: &[Entry], path: &str) -> crate::helpers::HelperResult {
    let (_root, base) = prepare_base(entries)?;
    let joined = base.join(path);
    let within = paths::is_within(&joined, &base)?;
    Ok(serde_json::Value::Bool(within))
}

fn run_join_within(base_dir: &str, parts: &[String]) -> crate::helpers::HelperResult {
    let parts_ref: Vec<&str> = parts.iter().map(String::as_str).collect();
    let Some(joined) = paths::join_within(Path::new(base_dir), &parts_ref) else {
        return Ok(serde_json::Value::Null);
    };
    let text = joined
        .to_str()
        .ok_or_else(|| Invalid("joined path is not valid UTF-8".to_owned()))?;
    Ok(serde_json::Value::String(text.to_owned()))
}

fn run_detect(header_hex: &str) -> crate::helpers::HelperResult {
    let bytes = hex::decode(header_hex).map_err(|error| Invalid(error.to_string()))?;
    let Some(media_type) = images::detect_local_image_media_type(&bytes) else {
        return Ok(serde_json::Value::Null);
    };
    Ok(serde_json::Value::String(media_type.to_owned()))
}

fn run_resolve(entries: &[Entry], candidate: &str) -> crate::helpers::HelperResult {
    let root = TempDir::new()?;
    fixture::materialize(root.path(), entries)?;
    let root_text = root
        .path()
        .to_str()
        .ok_or_else(|| Invalid("temporary directory path is not valid UTF-8".to_owned()))?;
    let resolved_candidate = candidate.replace("<root>", root_text);
    let logger = CapturingLogger::default();
    let outcome = images::resolve_local_image(&resolved_candidate, None, &logger);
    let (media_type, relative_path) = match outcome {
        Some(image) => {
            let canonical_root = soft_canonicalize(root.path())?;
            let relative = fixture::relative_to(&canonical_root, &image.path);
            (Some(image.media_type), relative)
        }
        None => (None, None),
    };
    Ok(serde_json::json!({
        "media_type": media_type,
        "relative_path": relative_path,
        "log_level": logger.level.get(),
    }))
}

#[derive(Default)]
struct CapturingLogger {
    level: Cell<Option<&'static str>>,
}

impl AssetLogger for CapturingLogger {
    fn debug(&self, _message: &str) {
        self.level.set(Some("debug"));
    }

    fn warning(&self, _message: &str) {
        self.level.set(Some("warning"));
    }
}

struct Invalid(String);

fn classified_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

impl From<std::io::Error> for Box<crate::helpers::HelperError> {
    fn from(error: std::io::Error) -> Self {
        classified_error("OSError", error.to_string())
    }
}

impl From<Invalid> for Box<crate::helpers::HelperError> {
    fn from(Invalid(message): Invalid) -> Self {
        classified_error("ValueError", message)
    }
}

impl From<PathError> for Box<crate::helpers::HelperError> {
    fn from(error: PathError) -> Self {
        match error {
            PathError::Io(source) => Self::from(source),
            PathError::NullByte | PathError::Escaped { .. } => {
                Self::from(Invalid(error.to_string()))
            }
        }
    }
}

impl From<FilenameError> for Box<crate::helpers::HelperError> {
    fn from(error: FilenameError) -> Self {
        Self::from(Invalid(error.to_string()))
    }
}
