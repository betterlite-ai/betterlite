use betterlite_openai::voyage_embedding::{
    VoyageEmbeddingError, VoyageEmbeddingVariant, decode_voyage_embedding_reply, endpoint,
    is_contextual_model, request,
};
use serde::Deserialize;
use serde_json::{Map, Value};

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
            "is_contextual": is_contextual_model(&input.contextual.model),
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
    Ok(serde_json::json!({
        "url": endpoint(call.base.as_deref(), variant),
        "request": request(&call.model, call.input.clone(), &call.options, variant),
        "response": response,
    }))
}

#[path = "voyage_contextual/tests.rs"]
mod tests;
