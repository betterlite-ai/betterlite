use betterlite_prompts::arguments::json::Json;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenApiToolOutput {
    Text(String),
    Json { value: Json, original_text: String },
}

// LiteLLM's parse_http_body: blank, invalid JSON or JSON null stay text; anything else is JSON plus its text
pub fn parse_http_body(body: &str) -> OpenApiToolOutput {
    if body.trim().is_empty() {
        return OpenApiToolOutput::Text(body.to_owned());
    }
    let parsed: Json = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => return OpenApiToolOutput::Text(body.to_owned()),
    };
    if parsed == Json::Null {
        return OpenApiToolOutput::Text(body.to_owned());
    }
    OpenApiToolOutput::Json {
        value: parsed,
        original_text: body.to_owned(),
    }
}
