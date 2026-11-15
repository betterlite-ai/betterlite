use crate::databricks_auth::error::{DatabricksError, SDK_API_BASE};
use crate::databricks_auth::url_parts;

pub fn get_api_base(
    api_base: Option<&str>,
    use_ai_gateway: bool,
) -> Result<String, DatabricksError> {
    let Some(base) = api_base else {
        return Err(DatabricksError {
            status_code: 400,
            message: SDK_API_BASE.to_owned(),
            headers: None,
        });
    };
    if !use_ai_gateway {
        return Ok(base.to_owned());
    }
    let normalized = base.trim_end_matches('/');
    if normalized.ends_with("/ai-gateway/mlflow/v1") {
        return Ok(normalized.to_owned());
    }
    if let Some(host) = normalized.strip_suffix("/serving-endpoints") {
        return Ok(format!("{host}/ai-gateway/mlflow/v1"));
    }
    if matches!(url_parts::path(normalized), "" | "/") {
        return Ok(format!("{normalized}/ai-gateway/mlflow/v1"));
    }
    // only the matching branches use the trimmed copy, so a non-matching base keeps its trailing slash
    Ok(base.to_owned())
}
