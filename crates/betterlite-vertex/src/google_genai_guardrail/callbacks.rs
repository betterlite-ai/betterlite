use std::error::Error;
use std::future::Future;
use std::pin::Pin;

use betterlite_prompts::arguments::json::{Json, Object};

use super::records::{request_texts, response_texts, write_texts};
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

// The two output shapes this handler guards: a whole response record, or the raw frames a stream has produced so far
pub enum GuardedOutput<'data> {
    Response(&'data mut Object),
    Frames(&'data [Json]),
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
    object
        .get("model")
        .and_then(Json::as_str)
        .map(str::to_owned)
}

async fn apply<Guardrail>(
    guardrail: &mut Guardrail,
    texts: Vec<String>,
    model: Option<String>,
    request_data: &Object,
    input_type: InputType,
) -> Result<Option<Vec<String>>, Guardrail::Failure>
where
    Guardrail: GenaiGuardrail,
{
    guardrail
        .apply_guardrail(
            GuardrailInputs { texts, model },
            GuardrailCall {
                request_data,
                input_type,
            },
        )
        .await
}

pub async fn process_request<Guardrail>(
    data: &mut Object,
    guardrail: &mut Guardrail,
) -> Result<(), Guardrail::Failure>
where
    Guardrail: GenaiGuardrail,
{
    let (locations, texts) = request_texts(data);
    if texts.is_empty() {
        tracing::debug!("Google GenAI guardrail: no request text found, skipping");
        return Ok(());
    }
    let model = model_value(data);
    if let Some(replacements) = apply(guardrail, texts, model, data, InputType::Request).await? {
        write_texts(data, &locations, &replacements);
    }
    Ok(())
}

pub async fn process_output<Guardrail>(
    output: GuardedOutput<'_>,
    request_data: Option<&Object>,
    user_metadata: &Object,
    guardrail: &mut Guardrail,
) -> Result<(), Guardrail::Failure>
where
    Guardrail: GenaiGuardrail,
{
    match output {
        GuardedOutput::Response(response) => {
            let (locations, texts) = response_texts(response);
            if texts.is_empty() {
                tracing::debug!("Google GenAI guardrail: no response text found, skipping");
                return Ok(());
            }
            let context = Json::Object(response.clone());
            let merged = merged_request_data(request_data, user_metadata, "response", context);
            let model = model_value(&merged);
            if let Some(replacements) =
                apply(guardrail, texts, model, &merged, InputType::Response).await?
            {
                write_texts(response, &locations, &replacements);
            }
        }
        GuardedOutput::Frames(frames) => {
            let text = accumulated_stream_text(frames);
            if text.is_empty() {
                return Ok(());
            }
            let context = Json::List(frames.to_vec());
            let merged =
                merged_request_data(request_data, user_metadata, "responses_so_far", context);
            apply(guardrail, vec![text], None, &merged, InputType::Response).await?;
        }
    }
    Ok(())
}

mod tests;
