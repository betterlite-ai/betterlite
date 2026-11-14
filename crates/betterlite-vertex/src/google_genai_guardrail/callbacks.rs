use std::error::Error;
use std::future::Future;
use std::pin::Pin;

use betterlite_prompts::arguments::json::{Json, Object};

use super::records::{Location, request_texts, response_texts, write_texts};
use super::sse::accumulated_stream_text;

pub type ApplyFuture<'call, Failure> =
    Pin<Box<dyn Future<Output = Result<Option<Vec<String>>, Failure>> + Send + 'call>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputType {
    Request,
    Response,
}

// LiteLLM's GenericGuardrailAPIInputs reduced to the keys this handler sets
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardrailInputs {
    pub texts: Vec<String>,
    pub model: Option<String>,
}

pub struct GuardrailCall<'data> {
    pub request_data: &'data Object,
    pub input_type: InputType,
}

// LiteLLM's CustomGuardrail.apply_guardrail, with the logging object left to the caller that records the invocation
pub trait GenaiGuardrail: Send {
    type Failure: Error + Send + Sync + 'static;

    fn apply_guardrail<'call>(
        &'call mut self,
        inputs: GuardrailInputs,
        call: GuardrailCall<'call>,
    ) -> ApplyFuture<'call, Self::Failure>;
}

// LiteLLM's _merged_request_data: supplied keys win, and only an absent key takes the current value
#[must_use]
pub fn merged_request_data(
    request_data: Option<&Object>,
    user_metadata: &Object,
    context_key: &str,
    context_value: Json,
) -> Object {
    let mut merged = request_data.cloned().unwrap_or_default();
    if !merged.contains_key(context_key) {
        merged.insert(context_key.to_owned(), context_value);
    }
    if !user_metadata.is_empty() && !merged.contains_key("litellm_metadata") {
        merged.insert(
            "litellm_metadata".to_owned(),
            Json::Object(user_metadata.clone()),
        );
    }
    merged
}

fn model_value(object: &Object) -> Option<String> {
    if let Some(Json::Text(model)) = object.get("model") {
        return Some(model.clone());
    }
    None
}

async fn apply<Guardrail: GenaiGuardrail>(
    guardrail: &mut Guardrail,
    inputs: GuardrailInputs,
    request_data: &Object,
    input_type: InputType,
) -> Result<Option<Vec<String>>, Guardrail::Failure> {
    guardrail
        .apply_guardrail(
            inputs,
            GuardrailCall {
                request_data,
                input_type,
            },
        )
        .await
}

pub async fn process_request<Guardrail: GenaiGuardrail>(
    data: &mut Object,
    guardrail: &mut Guardrail,
) -> Result<(), Guardrail::Failure> {
    let (locations, texts) = request_texts(data);
    if texts.is_empty() {
        tracing::debug!("Google GenAI guardrail: no request text found, skipping");
        return Ok(());
    }
    let inputs = GuardrailInputs {
        texts,
        model: model_value(data),
    };
    let returned = apply(guardrail, inputs, data, InputType::Request).await?;
    if let Some(returned) = returned {
        write_texts(data, &locations, &returned);
    }
    Ok(())
}

pub async fn process_response<Guardrail: GenaiGuardrail>(
    response: &mut Object,
    request_data: Option<&Object>,
    user_metadata: &Object,
    guardrail: &mut Guardrail,
) -> Result<(), Guardrail::Failure> {
    let (locations, texts) = response_texts(response);
    if texts.is_empty() {
        tracing::debug!("Google GenAI guardrail: no response text found, skipping");
        return Ok(());
    }
    let merged = merged_request_data(
        request_data,
        user_metadata,
        "response",
        Json::Object(response.clone()),
    );
    let inputs = GuardrailInputs {
        texts,
        model: model_value(&merged),
    };
    let returned = apply(guardrail, inputs, &merged, InputType::Response).await?;
    if let Some(returned) = returned {
        write_texts(response, &locations, &returned);
    }
    Ok(())
}

pub async fn process_stream<Guardrail: GenaiGuardrail>(
    frames: &[Json],
    request_data: Option<&Object>,
    user_metadata: &Object,
    guardrail: &mut Guardrail,
) -> Result<(), Guardrail::Failure> {
    let accumulated = accumulated_stream_text(frames);
    if accumulated.is_empty() {
        return Ok(());
    }
    let context = Json::List(frames.to_vec());
    let merged = merged_request_data(request_data, user_metadata, "responses_so_far", context);
    apply(
        guardrail,
        GuardrailInputs {
            texts: vec![accumulated],
            model: None,
        },
        &merged,
        InputType::Response,
    )
    .await?;
    Ok(())
}

mod tests;
