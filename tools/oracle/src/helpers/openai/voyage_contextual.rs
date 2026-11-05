use betterlite_openai::voyage_embedding::{
    VoyageEmbeddingError, VoyageEmbeddingVariant, decode_voyage_embedding_reply, endpoint, request,
    voyage_credentials, voyage_headers,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    text: Call,
    contextual: Call,
}

#[derive(Deserialize)]
struct Call {
    model: String,
    input: Value,
    #[serde(default)]
    options: Map<String, Value>,
    base: Option<String>,
    reply: Option<Reply>,
    api_key: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    headers: Map<String, Value>,
}

#[derive(Deserialize)]
struct Reply {
    body: String,
    status: u16,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let text_result = transform(&input.text, VoyageEmbeddingVariant::Text)?;
        let contextual_reply = transform(&input.contextual, VoyageEmbeddingVariant::Contextual)?;
        let contextual_result = serde_json::json!({
            "url": contextual_reply.get("url"),
            "request": contextual_reply.get("request"),
            "response": contextual_reply.get("response"),
            "headers": contextual_reply.get("headers"),
            "is_contextual": VoyageEmbeddingVariant::for_model(&input.contextual.model) == VoyageEmbeddingVariant::Contextual,
        });
        Ok(serde_json::json!({"text": text_result, "contextual": contextual_result}))
    }
}

fn transform(call: &Call, variant: VoyageEmbeddingVariant) -> crate::helpers::HelperResult {
    let response = call
        .reply
        .as_ref()
        .map(|reply| decode_voyage_embedding_reply(reply.body.as_bytes(), reply.status, variant))
        .transpose()
        .map_err(|error| match error {
            VoyageEmbeddingError::Decode {
                status, message, ..
            } => Box::new(crate::helpers::HelperError {
                class: "VoyageError".to_owned(),
                status: Some(status),
                error_type: None,
                param: None,
                code: None,
                message,
            }),
        })?;
    let authorization =
        voyage_credentials(call.api_key.as_deref(), |name| call.env.get(name).cloned());
    Ok(serde_json::json!({
        "headers": voyage_headers(call.headers.clone(), authorization),
        "url": endpoint(call.base.as_deref(), variant),
        "request": request(&call.model, call.input.clone(), &call.options, variant),
        "response": response,
    }))
}

#[path = "voyage_contextual/tests.rs"]
mod tests;
