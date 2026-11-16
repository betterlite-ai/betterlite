use serde_json::Value;

use super::error::WatsonxChatError;
use crate::watsonx_auth::environment::WatsonxEnvironment;
use crate::watsonx_auth::types::WatsonxParams;
use crate::watsonx_auth::urls::{add_api_version_to_url, get_base_url};

pub fn get_complete_url(
    api_base: Option<&str>,
    model: &str,
    stream: bool,
    options: &mut WatsonxParams,
    environment: &WatsonxEnvironment,
) -> Result<String, WatsonxChatError> {
    let base = get_base_url(api_base, environment)?;
    let suffix = if stream { "chat_stream" } else { "chat" };
    let endpoint = model.strip_prefix("deployment/").map_or_else(
        || format!("/ml/v1/text/{suffix}"),
        |deployment_id| format!("/ml/v1/deployments/{deployment_id}/text/{suffix}"),
    );
    let api_version = match options.remove("api_version") {
        None | Some(Value::Null) => None,
        Some(Value::String(version)) => Some(version),
        Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
            return Err(WatsonxChatError::InvalidField {
                field: "api_version",
            });
        }
    };
    let url = format!("{}{endpoint}", base.trim_end_matches('/'));
    Ok(add_api_version_to_url(&url, api_version.as_deref()))
}
