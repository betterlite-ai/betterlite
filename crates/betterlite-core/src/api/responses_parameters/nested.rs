use betterlite_context::non_default_params::Params;
use betterlite_paths::paths::delete::remove;
use betterlite_paths::paths::{Document, PathLimits};

use super::error::ResponsesParametersError;

const DROP_PATH_LIMITS: PathLimits = PathLimits {
    max_path_bytes: 512,
    max_depth: 32,
    max_selectors: 32,
    max_visits: 4096,
};

pub fn apply_nested_drop_params(
    params: Params,
    additional_drop_params: &[String],
) -> Result<Params, ResponsesParametersError> {
    let nested: Vec<&String> = additional_drop_params
        .iter()
        .filter(|path| path.contains('.') || path.contains('['))
        .collect();
    if nested.is_empty() {
        return Ok(params);
    }
    let json = serde_json::to_value(&params)
        .map_err(|source| ResponsesParametersError::Json { source })?;
    let mut document = Document { json };
    for path in nested {
        document = remove(&DROP_PATH_LIMITS, &document, path).map_err(|source| {
            ResponsesParametersError::DropPath {
                path: path.clone(),
                source,
            }
        })?;
    }
    serde_json::from_value(document.json)
        .map_err(|source| ResponsesParametersError::Json { source })
}
