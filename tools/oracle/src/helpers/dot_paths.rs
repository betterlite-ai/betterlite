use betterlite_paths::paths::delete::remove;
use betterlite_paths::paths::lookup::find_or;
use betterlite_paths::paths::{Document, PathLimits};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum DotPathsInput {
    Get {
        data: serde_json::Value,
        key_path: String,
        default: serde_json::Value,
    },
    Delete {
        data: serde_json::Value,
        path: String,
    },
}

const LIMITS: PathLimits = PathLimits {
    max_path_bytes: 8192,
    max_depth: 256,
    max_selectors: 256,
    max_visits: 100_000,
};

pub(crate) fn invoke(input: &DotPathsInput) -> Result<serde_json::Value, Box<super::HelperError>> {
    let result = match input {
        DotPathsInput::Get {
            data,
            key_path,
            default,
        } => find_or(
            &LIMITS,
            &Document { json: data.clone() },
            key_path,
            Document {
                json: default.clone(),
            },
        ),
        DotPathsInput::Delete { data, path } => {
            remove(&LIMITS, &Document { json: data.clone() }, path)
        }
    };
    result.map(|document| document.json).map_err(|error| {
        Box::new(super::HelperError {
            class: "PathError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}
