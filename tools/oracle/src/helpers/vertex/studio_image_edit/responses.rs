use betterlite_http::proxy::UpstreamResponse;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_vertex::studio_image_edit::response::transform_image_edit_response;
use betterlite_vertex::vertex_gemini_images::response::GeneratedImage;
use http::header::HeaderMap;
use serde_json::{Value, json};

use super::errors::edit_failure;
use super::ops::ResponseCase;

pub(super) fn response_value(case: &ResponseCase) -> Result<Value, Box<HelperError>> {
    let body = serde_json::to_vec(&case.body).or_config_error()?;
    let response = transform_image_edit_response(&UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body,
    })
    .map_err(|error| edit_failure(&error))?;
    let usage = match response.usage.as_ref() {
        Some(usage) => serde_json::to_value(usage).or_config_error()?,
        None => Value::Null,
    };
    Ok(json!({
        "images": response
            .data
            .iter()
            .map(image_value)
            .collect::<Vec<Value>>(),
        "usage": usage,
    }))
}

fn image_value(image: &GeneratedImage) -> Value {
    json!({
        "b64_json": image.b64_json.as_deref(),
        "thought_signature": image
            .provider_specific_fields
            .as_ref()
            .map(|fields| fields.thought_signature.as_str()),
    })
}
