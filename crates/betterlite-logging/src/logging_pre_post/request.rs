use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::args::RawBody;
use super::masking::{mask_api_base, masked_headers};
use crate::python_json;

pub struct CurlRequest<'call> {
    pub api_base: &'call str,
    pub headers: &'call IndexMap<String, Json>,
    pub request_str: Option<&'call str>,
    pub data: &'call RawBody,
}

#[must_use]
pub fn raw_request_body(data: Option<&RawBody>) -> Json {
    let message = match data {
        None => "Received empty dictionary for raw request body".to_owned(),
        Some(RawBody::Object(fields)) => return Json::Object(fields.clone()),
        Some(RawBody::Text(text)) => match serde_json::from_str::<Json>(text) {
            Ok(parsed) => return parsed,
            Err(_) => format!("Unable to parse raw request body. Got - {text}"),
        },
    };
    Json::Object(IndexMap::from([("error".to_owned(), Json::Text(message))]))
}

#[must_use]
pub fn curl_command(request: &CurlRequest<'_>) -> String {
    if let Some(request_str) = request.request_str {
        return format!("\nRequest Sent from LiteLLM:\n{request_str}");
    }
    let formatted = masked_headers(request.headers, false)
        .iter()
        .map(|(name, value)| {
            let shown = if let Json::Text(text) = value {
                text.clone()
            } else {
                python_json::dumps(value)
            };
            format!("-H '{name}: {shown}'")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let header_line = if formatted.trim().is_empty() {
        String::new()
    } else {
        format!("{formatted} \\\n")
    };
    let body = match request.data {
        RawBody::Text(text) => text.clone(),
        RawBody::Object(fields) => python_json::dumps_object(fields),
    };
    format!(
        "\n\nPOST Request Sent from LiteLLM:\ncurl -X POST \\\n{} \\\n{header_line}-d '{body}'\n",
        mask_api_base(request.api_base)
    )
}
