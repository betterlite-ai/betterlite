use betterlite_foundry::image_edits::reply::transform_mai_edit_response;
use betterlite_foundry::mai_images::reply::{ImageLogging, PostCall};
use secrecy::ExposeSecret as _;
use serde_json::{Map, Value, json};

use super::error_value;
use super::input::ReplyStep;

const CLOCK: i64 = 1_700_000_000;

#[derive(Default)]
struct Recorder {
    response: Option<Value>,
}

impl ImageLogging for Recorder {
    fn post_call(&mut self, call: PostCall<'_>) {
        self.response = Some(json!({
            "input": call.input,
            "api_key": call.api_key.map(|key| key.expose_secret().to_owned()),
            "original_response": call.original_response,
            "complete_input_dict": call.complete_input_dict,
        }));
    }
}

pub(super) fn reply_step(step: &ReplyStep) -> Value {
    let mut recorder = Recorder::default();
    match transform_mai_edit_response(step.body.as_bytes(), step.status, CLOCK, &mut recorder) {
        Ok(reply) => {
            // only the present image fields are projected, so an absent one compares like LiteLLM's exclude_none
            let data: Vec<Value> = reply
                .data
                .iter()
                .map(|datum| {
                    let mut fields = Map::new();
                    if let Some(encoded) = &datum.b64_json {
                        fields.insert(String::from("b64_json"), json!(encoded));
                    }
                    if let Some(url) = &datum.url {
                        fields.insert(String::from("url"), json!(url));
                    }
                    if let Some(prompt) = &datum.revised_prompt {
                        fields.insert(String::from("revised_prompt"), json!(prompt));
                    }
                    Value::Object(fields)
                })
                .collect();
            json!({
                "created": reply.created,
                "data": data,
                "size": reply.size,
                "usage": {
                    "input_tokens": reply.usage.input_tokens,
                    "input_tokens_details": {
                        "image_tokens": reply.usage.input_tokens_details.image_tokens,
                        "text_tokens": reply.usage.input_tokens_details.text_tokens,
                    },
                    "output_tokens": reply.usage.output_tokens,
                    "total_tokens": reply.usage.total_tokens,
                },
                "logged": recorder.response,
            })
        }
        Err(error) => error_value(&error),
    }
}
