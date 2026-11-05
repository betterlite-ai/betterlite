mod request;
mod scrub;

use betterlite_openai::chat::response::ChatChoice;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::chat::{ChatRequest, ToolCall};
use betterlite_openai::embedding::{EmbeddingResponse, EmbeddingVector};
use betterlite_openai::stream::chunk::Choice;
use betterlite_openai::stream::chunk::Chunk;
use betterlite_redact::secret::Redactor;
use serde::{Serialize, de::DeserializeOwned};

const REDACTED: &str = "redacted-by-litellm";

pub trait Payload: Sized + Serialize + DeserializeOwned {
    fn snapshot(&self, private: bool, redactor: &Redactor) -> Result<Self, SnapshotError> {
        let mut output = self.copy_for_callback()?;
        redact(output.view(), private, redactor)?;
        Ok(output)
    }

    fn copy_for_callback(&self) -> Result<Self, SnapshotError> {
        Ok(serde_json::from_slice(&serde_json::to_vec(self)?)?)
    }

    fn view(&mut self) -> View<'_>;
}

pub enum View<'payload> {
    Request(&'payload mut ChatRequest),
    Response(&'payload mut ChatResponse),
    Stream(&'payload mut Chunk),
    Embedding(&'payload mut EmbeddingResponse),
}

impl Payload for ChatRequest {
    fn copy_for_callback(&self) -> Result<Self, SnapshotError> {
        Ok(self.clone())
    }

    fn view(&mut self) -> View<'_> {
        View::Request(self)
    }
}

impl Payload for ChatResponse {
    fn copy_for_callback(&self) -> Result<Self, SnapshotError> {
        Ok(self.clone())
    }

    fn view(&mut self) -> View<'_> {
        View::Response(self)
    }
}

impl Payload for Chunk {
    fn view(&mut self) -> View<'_> {
        View::Stream(self)
    }
}

impl Payload for EmbeddingResponse {
    fn copy_for_callback(&self) -> Result<Self, SnapshotError> {
        Ok(self.clone())
    }

    fn view(&mut self) -> View<'_> {
        View::Embedding(self)
    }
}

fn redact(view: View<'_>, private: bool, redactor: &Redactor) -> Result<(), SnapshotError> {
    match view {
        View::Request(request) => request::redact(request, private, redactor),
        View::Response(response) => {
            scrub::identifiers(
                [&mut response.id, &mut response.object, &mut response.model],
                [&mut response.system_fingerprint, &mut response.service_tier],
                redactor,
            );
            response
                .choices
                .iter_mut()
                .try_for_each(|choice| answer(choice, private, redactor))
        }
        View::Stream(stream) => {
            scrub::identifiers(
                [&mut stream.id, &mut stream.object, &mut stream.model],
                [&mut stream.system_fingerprint, &mut stream.service_tier],
                redactor,
            );
            for choice in &mut stream.choices {
                chunk(choice, private, redactor);
            }
            Ok(())
        }
        View::Embedding(response) => {
            embedding(response, private, redactor);
            Ok(())
        }
    }
}

fn embedding(response: &mut EmbeddingResponse, private: bool, redactor: &Redactor) {
    if private {
        response.data.clear();
    }
    response.model = redactor.redact_string(&response.model);
    response.object = redactor.redact_string(&response.object);
    for record in &mut response.data {
        record.object = redactor.redact_string(&record.object);
        if let EmbeddingVector::Base64(encoded) = &mut record.embedding {
            *encoded = redactor.redact_string(encoded);
        }
    }
}

fn text(value: &mut Option<String>, private: bool, redactor: &Redactor) {
    if let Some(current) = value {
        *current = content(current, private, redactor);
    }
}

fn answer(
    choice: &mut ChatChoice,
    private: bool,
    redactor: &Redactor,
) -> Result<(), SnapshotError> {
    text(&mut choice.finish_reason, false, redactor);
    choice.message.role = redactor.redact_string(&choice.message.role);
    scrub::passthrough(&mut choice.provider_specific_fields, redactor)?;
    scrub::passthrough(&mut choice.logprobs, redactor)?;
    scrub::passthrough(&mut choice.message.audio, redactor)?;
    scrub::passthrough(&mut choice.message.annotations, redactor)?;
    text(&mut choice.message.content, private, redactor);
    text(&mut choice.message.refusal, private, redactor);
    text(
        &mut choice.message.provider_specific_fields.refusal,
        private,
        redactor,
    );
    calls(&mut choice.message.tool_calls, private, redactor);
    if private {
        choice.logprobs = None;
        choice.message.audio = None;
        choice.message.annotations = None;
    }

    Ok(())
}

fn chunk(choice: &mut Choice, private: bool, redactor: &Redactor) {
    text(&mut choice.finish_reason, false, redactor);
    text(&mut choice.delta.role, false, redactor);
    if let Some(probabilities) = &mut choice.logprobs {
        scrub::logprobs(probabilities, redactor);
    }
    text(&mut choice.delta.content, private, redactor);
    text(&mut choice.delta.refusal, private, redactor);
    if let Some(calls) = &mut choice.delta.tool_calls {
        for call in calls {
            text(&mut call.id, false, redactor);
            text(&mut call.kind, false, redactor);
            if let Some(function) = &mut call.function {
                text(&mut function.name, false, redactor);
                function.arguments = content(&function.arguments, private, redactor);
            }
        }
    }
    if let Some(function) = &mut choice.delta.function_call {
        text(&mut function.name, false, redactor);
        function.arguments = content(&function.arguments, private, redactor);
    }
    if private {
        choice.logprobs = None;
    }
}

fn calls(values: &mut Option<Vec<ToolCall>>, private: bool, redactor: &Redactor) {
    for call in values.iter_mut().flatten() {
        for value in [&mut call.id, &mut call.kind, &mut call.function.name] {
            *value = redactor.redact_string(value);
        }
        call.function.arguments = content(&call.function.arguments, private, redactor);
    }
}

fn content(value: &str, private: bool, redactor: &Redactor) -> String {
    if private {
        REDACTED.to_owned()
    } else {
        redactor.redact_string(value)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("invalid snapshot passthrough: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}

mod tests;
