use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tracing::{debug, warn};

// litellm.integrations.langfuse.LangFuseLogger.add_metadata_from_header
#[must_use]
pub fn add_metadata_from_header(
    litellm_params: Option<&Json>,
    mut metadata: IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let Some(Json::Object(params)) = litellm_params else {
        return metadata;
    };
    let Some(Json::Object(request)) = params.get("proxy_server_request") else {
        return metadata;
    };
    let Some(Json::Object(headers)) = request.get("headers") else {
        return metadata;
    };
    for (header, value) in headers {
        let Some(key) = header.strip_prefix("langfuse_") else {
            continue;
        };
        if metadata.contains_key(key) {
            warn!("Overwriting Langfuse `{key}` from request header");
        } else {
            debug!("Found Langfuse `{key}` in request header");
        }
        metadata.insert(key.to_owned(), value.clone());
    }
    metadata
}
